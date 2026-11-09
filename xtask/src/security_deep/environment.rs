use super::*;

impl PrivateExecutionContext {
    pub(super) fn require_exact(&self) -> Result<()> {
        let source = self
            .marker
            .strip_prefix(WRAPPER_MARKER_PREFIX)
            .context("private wrapper selector is malformed")?;
        ensure!(
            source.len() == 40
                && source
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "private wrapper selector has an invalid source identity"
        );
        let scratch = self
            .pycache
            .parent()
            .context("private Python bytecode directory has no scratch parent")?;
        require_scratch_cleanup_target(scratch)?;
        ensure!(
            self.scan_root == scratch.join("scans")
                && self.plugin_mirror == scratch.join("plugin")
                && self.pycache == scratch.join("pycache")
                && self.projected_auth == scratch.join("codex-home").join("auth.json"),
            "private wrapper context escaped owned scratch"
        );
        let (bytes, digest) = measure_file(
            &self.python_real,
            MAX_EXECUTABLE_BYTES,
            "private Python runtime",
        )?;
        ensure!(
            bytes == PYTHON_BYTES && sha256::hex(&digest) == PYTHON_SHA256,
            "private Python runtime differs from the qualified executable"
        );
        ensure!(
            self.python_real.parent() == Some(self.python_runtime.root.as_path())
                && identify_python_runtime(&self.python_real)? == self.python_runtime,
            "private Python runtime tree differs from the qualified runtime"
        );
        let (bytes, digest) = measure_file(
            &self.artifact_node.path,
            MAX_EXECUTABLE_BYTES,
            "private artifact Node runtime",
        )?;
        ensure!(
            bytes == self.artifact_node.bytes
                && sha256::hex(&digest) == self.artifact_node.sha256
                && self.artifact_node.bytes == NODE_BYTES
                && self.artifact_node.sha256 == NODE_SHA256
                && self.artifact_node.version == NODE_VERSION,
            "private artifact Node runtime differs from the qualified executable"
        );
        ensure!(
            self.python_wrapper != self.python_real,
            "private Python wrapper resolves to its real runtime"
        );
        ensure!(
            self.live_auth.is_absolute()
                && self.projected_auth.is_absolute()
                && self.live_auth != self.projected_auth
                && self.live_auth.file_name() == Some(OsStr::new("auth.json"))
                && self.projected_auth.file_name() == Some(OsStr::new("auth.json")),
            "private wrapper credential deny paths differ"
        );
        self.shell_environment.require_current()?;
        Ok(())
    }

    pub(super) fn artifact_overrides(&self) -> Result<Vec<String>> {
        let mut overrides: Vec<String> = [
            (WRAPPER_MARKER_ENV, self.marker.clone()),
            (
                PYTHON_WRAPPER_REAL_ENV,
                self.python_real
                    .to_str()
                    .context("private Python runtime path is not UTF-8")?
                    .to_string(),
            ),
            (
                PYTHON_WRAPPER_PLUGIN_ENV,
                self.plugin_mirror
                    .to_str()
                    .context("private plugin mirror path is not UTF-8")?
                    .to_string(),
            ),
            (
                PYTHON_WRAPPER_PYCACHE_ENV,
                self.pycache
                    .to_str()
                    .context("private Python bytecode path is not UTF-8")?
                    .to_string(),
            ),
            (
                PYTHON_WRAPPER_RUNTIME_ENV,
                render_python_runtime_policy(&self.python_runtime)?,
            ),
            (
                WRAPPER_REPOSITORY_ENV,
                self.repository
                    .to_str()
                    .context("private repository path is not UTF-8")?
                    .to_string(),
            ),
        ]
        .into_iter()
        .map(|(name, value)| {
            format!(
                "mcp_servers.cs_artifacts.env.{name}={}",
                quote_toml_string(&value)
            )
        })
        .collect();
        overrides.extend([
            "shell_environment_policy.inherit=\"none\"".to_string(),
            "shell_environment_policy.ignore_default_excludes=false".to_string(),
            self.shell_environment.set_override()?,
        ]);
        Ok(overrides)
    }
}

impl ShellEnvironmentPolicy {
    pub(super) fn qualify(tools: &Toolchain, git: &Path, scratch: &OwnedScratch) -> Result<Self> {
        Self::qualify_paths(
            &tools.node.path,
            &tools.python.path,
            &tools.codex.path,
            git,
            &scratch.temporary,
        )
    }

    pub(super) fn qualify_paths(
        node: &Path,
        python: &Path,
        codex: &Path,
        git: &Path,
        temporary: &Path,
    ) -> Result<Self> {
        ensure!(
            cfg!(windows),
            "private Codex shell environment requires Windows"
        );
        let supplied_system_root = PathBuf::from(
            std::env::var_os("SYSTEMROOT")
                .context("private Codex shell environment lacks SYSTEMROOT")?,
        );
        ensure!(
            supplied_system_root.is_absolute(),
            "private Codex SYSTEMROOT is relative"
        );
        let system_root = canonical_directory(&supplied_system_root, "Windows system root")?;
        let supplied_windows = PathBuf::from(
            std::env::var_os("WINDIR").context("private Codex shell environment lacks WINDIR")?,
        );
        ensure!(
            canonical_directory(&supplied_windows, "Windows directory")? == system_root,
            "private Codex WINDIR differs from SYSTEMROOT"
        );
        let system32 = canonical_directory(&system_root.join("System32"), "Windows System32")?;
        ensure!(
            system32 == system_root.join("System32"),
            "Windows System32 path is redirected"
        );
        let command_processor =
            require_exact_executable_path(&system32.join("cmd.exe"), "Windows command processor")?;
        let temporary = canonical_directory(temporary, "private Codex temporary directory")?;

        let mut search_directories = Vec::new();
        for (path, label) in [
            (node, "qualified Node runtime"),
            (python, "qualified Python runtime"),
            (codex, "qualified Codex runtime"),
            (git, "qualified Git runtime"),
        ] {
            let parent = path
                .parent()
                .with_context(|| format!("{label} has no parent"))?;
            ensure!(
                canonical_directory(parent, label)? == parent,
                "{label} parent is redirected"
            );
            search_directories.push(parent.to_path_buf());
        }
        search_directories.push(system32);
        let search_path = std::env::join_paths(search_directories)
            .context("building private Codex shell PATH")?;
        Ok(Self {
            system_root,
            command_processor,
            search_path,
            temporary,
        })
    }

    pub(super) fn set_override(&self) -> Result<String> {
        let fields = [
            (
                "SYSTEMROOT",
                self.path_text(&self.system_root, "SYSTEMROOT")?,
            ),
            ("WINDIR", self.path_text(&self.system_root, "WINDIR")?),
            (
                "COMSPEC",
                self.path_text(&self.command_processor, "COMSPEC")?,
            ),
            ("PATHEXT", WINDOWS_PATH_EXTENSIONS.to_string()),
            (
                "PATH",
                self.search_path
                    .to_str()
                    .context("private Codex shell PATH is not UTF-8")?
                    .to_string(),
            ),
            ("TEMP", self.path_text(&self.temporary, "TEMP")?),
            ("TMP", self.path_text(&self.temporary, "TMP")?),
        ];
        ensure!(
            fields
                .iter()
                .all(|(_, value)| { !value.is_empty() && !value.contains(['\r', '\n', '\0']) }),
            "private Codex shell environment contains a malformed value"
        );
        Ok(format!(
            "shell_environment_policy.set={{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!("{name}={}", quote_toml_string(value)))
                .collect::<Vec<_>>()
                .join(",")
        ))
    }

    pub(super) fn apply(&self, command: &mut Command) {
        command
            .env("SYSTEMROOT", &self.system_root)
            .env("WINDIR", &self.system_root)
            .env("COMSPEC", &self.command_processor)
            .env("PATHEXT", WINDOWS_PATH_EXTENSIONS)
            .env("PATH", &self.search_path)
            .env("TEMP", &self.temporary)
            .env("TMP", &self.temporary);
    }

    fn require_current(&self) -> Result<()> {
        for (name, expected) in [
            ("SYSTEMROOT", self.system_root.as_os_str()),
            ("WINDIR", self.system_root.as_os_str()),
            ("COMSPEC", self.command_processor.as_os_str()),
            ("PATHEXT", OsStr::new(WINDOWS_PATH_EXTENSIONS)),
            ("PATH", self.search_path.as_os_str()),
            ("TEMP", self.temporary.as_os_str()),
            ("TMP", self.temporary.as_os_str()),
        ] {
            ensure!(
                std::env::var_os(name).as_deref() == Some(expected),
                "private Codex shell environment {name} differs from its qualified value"
            );
        }
        Ok(())
    }

    fn path_text(&self, path: &Path, name: &str) -> Result<String> {
        path.to_str()
            .with_context(|| format!("private Codex shell {name} is not UTF-8"))
            .map(str::to_string)
    }
}
