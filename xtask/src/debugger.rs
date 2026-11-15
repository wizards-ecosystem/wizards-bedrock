use std::ffi::OsString;
use std::net::TcpListener;
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::cargo::{Profile, VIRTUAL_SOURCE_ROOT};
use crate::image::{self, Kernel};
use crate::paths::{Paths, RunDirectory};

const ENTRY_SYMBOL: &str = "kmain";

pub(crate) fn run(paths: &Paths) -> Result<()> {
    let gdb = std::env::var_os("GDB").unwrap_or_else(|| OsString::from("gdb"));
    let run = RunDirectory::create(paths)?;
    image::build(paths, Profile::Dev, Kernel::Shipped, &run.image())?;
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .context("choosing a GDB port")?
        .port();
    let mut machine = crate::qemu::debug(&run, port)?;
    println!("[xtask] paused QEMU; GDB will stop at {ENTRY_SYMBOL}, `quit` ends the VM");
    let source = paths.root().to_string_lossy().replace('\\', "/");
    let mut commands = Vec::new();
    for command in [
        "set pagination off".to_string(),
        "set confirm off".to_string(),
        format!("set substitute-path {VIRTUAL_SOURCE_ROOT} \"{source}\""),
        format!("target remote 127.0.0.1:{port}"),
        format!("hbreak {ENTRY_SYMBOL}"),
        "continue".to_string(),
    ] {
        commands.extend(["--ex".to_string(), command]);
    }
    let status = Command::new(&gdb)
        .args(["--quiet", "--nx", "--nh", "--se"])
        .arg(paths.kernel_elf(Profile::Dev))
        .args(&commands)
        .status();
    let _ = machine.kill();
    let _ = machine.wait();
    let status = status.with_context(|| format!("could not run {}", gdb.to_string_lossy()))?;
    ensure!(status.success(), "GDB exited unsuccessfully ({status})");
    Ok(())
}
