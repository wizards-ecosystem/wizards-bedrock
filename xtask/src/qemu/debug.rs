use std::fmt;

use super::*;

pub(crate) struct DebugMachine {
    child: Option<Child>,
    endpoint: String,
    _qmp: Qmp,
    _vars: RunVars,
    _coverage: RunDmaCoverage,
}

impl DebugMachine {
    pub(crate) fn start(paths: &Paths, image: &Path) -> Result<Self> {
        let installation = installation()?;
        let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
        let coverage = RunDmaCoverage::create(paths.target_dir(), Remapping::Present)?;

        let mut child = Command::new(&installation.emulator)
            .current_dir(vars.directory())
            .args(machine_args(Remapping::Present))
            .args(coverage.arguments())
            .args(["-S", "-display", "none", "-serial", "null", "-no-reboot"])
            .args(QMP_ON_STDIO)
            .args(drive_file(
                "if=pflash,format=raw,readonly=on",
                &installation.code_firmware,
            ))
            .args(drive_file("if=pflash,format=raw", vars.path()))
            .args(snapshot_drive_file("format=raw", image))
            .args(storage(paths.root())?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

        let started = (|| -> Result<(Qmp, String)> {
            let deadline = Instant::now() + DEBUG_CONTROL_TIMEOUT;
            let mut qmp = attach_qmp(&mut child, deadline, &NOT_CANCELLED)
                .context("negotiating the paused machine's QMP session")?;
            qmp.human("gdbserver tcp:127.0.0.1:0", deadline, &NOT_CANCELLED)
                .context("starting QEMU's dynamic GDB server")?;
            let report = qmp
                .human("info chardev", deadline, &NOT_CANCELLED)
                .context("querying QEMU's dynamic GDB endpoint")?;
            Ok((qmp, parse_debug_endpoint(&report)?))
        })();

        match started {
            Ok((qmp, endpoint)) => Ok(Self {
                child: Some(child),
                endpoint,
                _qmp: qmp,
                _vars: vars,
                _coverage: coverage,
            }),
            Err(error) => {
                let cleanup = stop_qemu(&mut child);
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => {
                        Err(anyhow!("{error:#}; QEMU cleanup also failed: {cleanup:#}"))
                    }
                }
            }
        }
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Stop and reap the machine before its private state is removed.
    pub(crate) fn stop(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        stop_qemu(child)?;
        self.child = None;
        Ok(())
    }
}

impl fmt::Debug for DebugMachine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugMachine")
            .field("child", &self.child)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl Drop for DebugMachine {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = stop_qemu(child);
        }
    }
}

const DEBUG_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const DEBUG_ENDPOINT_PREFIX: &str = "tcp:127.0.0.1:";

pub(super) fn parse_debug_endpoint(report: &str) -> Result<String> {
    let line = report
        .lines()
        .find(|line| line.trim_start().starts_with("gdb:"))
        .context("QEMU reported no GDB chardev")?;
    let tail = line
        .split_once(DEBUG_ENDPOINT_PREFIX)
        .map(|(_, tail)| tail)
        .context("QEMU's GDB chardev is not bound to IPv4 loopback")?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    let port = <u16 as core::str::FromStr>::from_str(&digits)
        .context("QEMU reported an invalid GDB port")?;
    if port == 0 {
        bail!("QEMU did not allocate a GDB port");
    }
    Ok(format!("127.0.0.1:{port}"))
}
