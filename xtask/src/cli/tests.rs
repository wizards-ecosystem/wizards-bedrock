use super::*;

fn parse_args(args: &[&str]) -> Result<Task> {
    parse(args.iter().map(OsString::from))
}

#[test]
fn no_arguments_is_an_error_not_run() {
    assert!(parse_args(&[]).is_err());
}

#[test]
fn builds_are_optimised_by_default() {
    assert_eq!(
        parse_args(&["build"]).unwrap(),
        Task::Build(Profile::Release)
    );
    assert_eq!(
        parse_args(&["build", "--debug"]).unwrap(),
        Task::Build(Profile::Dev)
    );
    assert_eq!(
        parse_args(&["image", "--release"]).unwrap(),
        Task::Image(Profile::Release)
    );
}

#[test]
fn help_is_reachable_every_way_a_user_might_ask() {
    for args in [
        vec!["help"],
        vec!["-h"],
        vec!["--help"],
        vec!["build", "--help"],
    ] {
        assert_eq!(parse_args(&args).unwrap(), Task::Help);
    }
}

#[test]
fn run_forwards_arguments_after_a_double_dash() {
    let task = parse_args(&["run", "--debug", "--", "-display", "none"]).unwrap();
    assert_eq!(
        task,
        Task::Run {
            profile: Profile::Dev,
            qemu_args: vec![OsString::from("-display"), OsString::from("none")],
        }
    );
}

#[test]
fn flags_after_a_double_dash_are_not_ours_to_read() {
    let task = parse_args(&["run", "--", "--debug"]).unwrap();
    assert_eq!(
        task,
        Task::Run {
            profile: Profile::Release,
            qemu_args: vec![OsString::from("--debug")],
        }
    );
}

#[test]
fn task_specific_flags_reach_their_task() {
    assert_eq!(
        parse_args(&["qualify-advisory"]).unwrap(),
        Task::QualifyAdvisory
    );
    assert_eq!(
        parse_args(&["fmt", "--check"]).unwrap(),
        Task::Fmt(FmtMode::Check)
    );
    assert_eq!(parse_args(&["fmt"]).unwrap(), Task::Fmt(FmtMode::Write));
    assert_eq!(
        parse_args(&["debug", "--check"]).unwrap(),
        Task::Debug(DebugMode::Check)
    );
    assert_eq!(
        parse_args(&["debug"]).unwrap(),
        Task::Debug(DebugMode::Interactive)
    );
    assert_eq!(
        parse_args(&["vmware"]).unwrap(),
        Task::Vmware {
            profile: Profile::Release,
            kernel: crate::image::Kernel::SelfTest,
        }
    );
    assert_eq!(
        parse_args(&["vmware", "--shipped"]).unwrap(),
        Task::Vmware {
            profile: Profile::Release,
            kernel: crate::image::Kernel::Shipped,
        }
    );
    assert_eq!(
        parse_args(&["qualify-vmware"]).unwrap(),
        Task::QualifyVmware {
            kernel: crate::image::Kernel::SelfTest,
        }
    );
    assert_eq!(
        parse_args(&["qualify-vmware", "--shipped"]).unwrap(),
        Task::QualifyVmware {
            kernel: crate::image::Kernel::Shipped,
        }
    );
    assert_eq!(
        parse_args(&["clean", "--all"]).unwrap(),
        Task::Clean(CleanScope::Everything)
    );
    assert_eq!(
        parse_args(&["clean"]).unwrap(),
        Task::Clean(CleanScope::Build)
    );
    assert_eq!(
        parse_args(&["check", "--message-format=json"]).unwrap(),
        Task::Check {
            message_format: Some("json".to_string())
        }
    );
}

#[test]
fn unknown_tasks_and_options_are_rejected() {
    assert!(parse_args(&["bulid"]).is_err());
    assert!(parse_args(&["build", "--fast"]).is_err());
}

#[test]
fn a_mistyped_task_or_run_option_says_what_was_meant() {
    let error = |args: &[&str]| parse_args(args).unwrap_err().to_string();
    assert_eq!(
        error(&["bulid"]),
        "unknown task `bulid`; did you mean `build`?"
    );
    assert_eq!(error(&["not-a-task"]), "unknown task `not-a-task`");
    for short in ["x", "", "-x", "rm"] {
        assert_eq!(error(&[short]), format!("unknown task `{short}`"));
    }
    assert_eq!(
        error(&["run", "-display"]),
        "unknown option `-display`; pass QEMU arguments after `--`"
    );
    assert_eq!(
        error(&["run", "--relase"]),
        "unknown option `--relase`; pass QEMU arguments after `--`"
    );
    assert_eq!(error(&["build", "-display"]), "unknown option `-display`");
}

#[test]
fn a_name_too_short_or_too_far_from_every_task_suggests_nothing() {
    let error = |name: &str| parse_args(&[name]).unwrap_err().to_string();
    assert_eq!(error("dc"), "unknown task `dc`");
    assert_eq!(error("qualify-vmw"), "unknown task `qualify-vmw`");
}

fn assert_refused_elsewhere(options: &[&str], consumers: &[&str]) {
    for task in [
        "build",
        "image",
        "run",
        "vmware",
        "check",
        "api-audit",
        "clippy",
        "doc",
        "fmt",
        "test",
        "test-boot",
        "boot-check",
        "degraded-check",
        "supply-chain",
        "verify-vendor",
        "update-sbom",
        "model-check",
        "baseline",
        "debug",
        "mutate",
        "ci",
        "evidence",
        "verify-evidence",
        "qualify-advisory",
        "qualify-mutants-grade",
        "qualify-security-deep",
        "qualify-vmware",
        "clean",
    ] {
        if consumers.contains(&task) {
            continue;
        }
        let mut arguments = vec![task];
        if matches!(task, "qualify-mutants-grade" | "qualify-security-deep") {
            arguments.push("--run");
        }
        assert!(
            parse_args(&arguments).is_ok(),
            "invalid fixture: {arguments:?}"
        );
        arguments.extend_from_slice(options);
        assert!(
            parse_args(&arguments).is_err(),
            "accepted misplaced option: {arguments:?}"
        );
    }
}

#[test]
fn profile_options_only_reach_tasks_with_a_build_profile() {
    for (option, profile) in [("--release", Profile::Release), ("--debug", Profile::Dev)] {
        for (task, expected) in [
            ("build", Task::Build(profile)),
            ("image", Task::Image(profile)),
            (
                "run",
                Task::Run {
                    profile,
                    qemu_args: Vec::new(),
                },
            ),
            (
                "vmware",
                Task::Vmware {
                    profile,
                    kernel: crate::image::Kernel::SelfTest,
                },
            ),
            ("test-boot", Task::TestBoot(profile)),
            ("boot-check", Task::BootCheck(profile)),
            ("degraded-check", Task::DegradedCheck(profile)),
        ] {
            assert_eq!(parse_args(&[task, option]).unwrap(), expected);
        }
        assert_refused_elsewhere(
            &[option],
            &[
                "build",
                "image",
                "run",
                "vmware",
                "test-boot",
                "boot-check",
                "degraded-check",
            ],
        );
    }
}

#[test]
fn check_options_cannot_dispatch_cleanup_or_another_unrelated_task() {
    assert_eq!(
        parse_args(&["fmt", "--check"]).unwrap(),
        Task::Fmt(FmtMode::Check)
    );
    assert_eq!(
        parse_args(&["debug", "--check"]).unwrap(),
        Task::Debug(DebugMode::Check)
    );
    let error = parse_args(&["clean", "--check"]).unwrap_err().to_string();
    assert_eq!(error, "`--check` does not apply to `clean`");
    assert_refused_elsewhere(&["--check"], &["fmt", "debug"]);
}

#[test]
fn all_options_only_expand_the_clean_task() {
    assert_eq!(
        parse_args(&["clean", "--all"]).unwrap(),
        Task::Clean(CleanScope::Everything)
    );
    assert_refused_elsewhere(&["--all"], &["clean"]);
}

#[test]
fn shipped_options_only_select_vmware_images() {
    assert_eq!(
        parse_args(&["vmware", "--shipped"]).unwrap(),
        Task::Vmware {
            profile: Profile::Release,
            kernel: crate::image::Kernel::Shipped,
        }
    );
    assert_eq!(
        parse_args(&["qualify-vmware", "--shipped"]).unwrap(),
        Task::QualifyVmware {
            kernel: crate::image::Kernel::Shipped,
        }
    );
    assert_refused_elsewhere(&["--shipped"], &["vmware", "qualify-vmware"]);
}

#[test]
fn message_format_options_only_reach_check_even_when_empty() {
    for (option, value) in [("--message-format=json", "json"), ("--message-format=", "")] {
        assert_eq!(
            parse_args(&["check", option]).unwrap(),
            Task::Check {
                message_format: Some(value.to_string()),
            }
        );
        assert_refused_elsewhere(&[option], &["check"]);
    }
}

#[test]
fn passthrough_is_run_only_even_when_the_separator_has_no_tail() {
    assert_eq!(
        parse_args(&["run", "--"]).unwrap(),
        Task::Run {
            profile: Profile::Release,
            qemu_args: Vec::new(),
        }
    );
    assert_refused_elsewhere(&["--"], &["run"]);
    assert_refused_elsewhere(&["--", "-display", "none"], &["run"]);
}

#[test]
fn help_and_repeated_ordinary_options_keep_their_existing_precedence() {
    for option in [
        "--release",
        "--debug",
        "--check",
        "--all",
        "--shipped",
        "--message-format=json",
    ] {
        assert_eq!(
            parse_args(&["clean", option, "--help"]).unwrap(),
            Task::Help
        );
    }
    assert_eq!(
        parse_args(&["build", "--debug", "--release"]).unwrap(),
        Task::Build(Profile::Release)
    );
    assert_eq!(
        parse_args(&["build", "--release", "--debug"]).unwrap(),
        Task::Build(Profile::Dev)
    );
    assert_eq!(
        parse_args(&["fmt", "--check", "--check"]).unwrap(),
        Task::Fmt(FmtMode::Check)
    );
    assert_eq!(
        parse_args(&["clean", "--all", "--all"]).unwrap(),
        Task::Clean(CleanScope::Everything)
    );
    assert_eq!(
        parse_args(&["vmware", "--shipped", "--shipped"]).unwrap(),
        Task::Vmware {
            profile: Profile::Release,
            kernel: crate::image::Kernel::Shipped,
        }
    );
    assert_eq!(
        parse_args(&["check", "--message-format=short", "--message-format=json"]).unwrap(),
        Task::Check {
            message_format: Some("json".to_string()),
        }
    );
    assert_eq!(
        parse_args(&["run", "--", "--help"]).unwrap(),
        Task::Run {
            profile: Profile::Release,
            qemu_args: vec![OsString::from("--help")],
        }
    );
}

/// A task the parser accepts but the help text never mentions is a task
/// nobody will run; one the help offers but the parser rejects is worse.
#[test]
fn every_task_the_parser_accepts_is_named_in_the_help() {
    // Derive this from the parser so the test cannot drift through a
    // second handwritten task list.
    let names: Vec<&str> = include_str!("../cli.rs")
        .lines()
        .filter(|line| {
            line.contains("=> Ok(Task::") || line.contains("=> match (run, begin, verify)")
        })
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|rest| rest.split_once('"').map(|(name, _)| name))
        .collect();
    assert!(
        names.len() > 10,
        "the parser was read as {} tasks, so the reading is wrong",
        names.len()
    );
    assert_eq!(names, TASK_NAMES, "TASK_NAMES drifted from the parser");

    let help = help();
    for name in names {
        let arguments = if name == "qualify-mutants-grade" {
            vec![name, "--run"]
        } else if name == "qualify-security-deep" {
            vec![name, "--begin"]
        } else {
            vec![name]
        };
        assert!(parse_args(&arguments).is_ok(), "{name} does not parse");
        assert!(
            help.contains(&format!("\n    {name} ")),
            "{name} is missing from the help text"
        );
    }
}

#[test]
fn the_doc_task_parses() {
    assert_eq!(parse_args(&["doc"]).unwrap(), Task::Doc);
}

#[test]
fn external_qualification_actions_are_typed_and_task_local() {
    assert_eq!(
        parse_args(&["qualify-mutants-grade", "--run"]).unwrap(),
        Task::QualifyMutantsGrade(MutantsGradeAction::Run)
    );
    assert_eq!(
        parse_args(&["qualify-security-deep", "--verify=scan"]).unwrap(),
        Task::QualifySecurityDeep(SecurityDeepAction::Verify(PathBuf::from("scan")))
    );
    assert_eq!(
        parse_args(&["qualify-security-deep", "--run"]).unwrap(),
        Task::QualifySecurityDeep(SecurityDeepAction::Run)
    );
    for task in ["qualify-mutants-grade", "qualify-security-deep"] {
        assert!(parse_args(&[task]).is_err(), "{task} ran with no action");
    }
    assert!(parse_args(&["qualify-mutants-grade", "--begin"]).is_err());
    assert!(parse_args(&["qualify-mutants-grade", "--verify=scan"]).is_err());
    assert!(parse_args(&["qualify-mutants-grade", "--run", "--begin"]).is_err());
    assert!(parse_args(&["qualify-mutants-grade", "--run", "--verify=scan"]).is_err());
    assert!(parse_args(&["qualify-security-deep", "--run", "--begin"]).is_err());
    assert!(parse_args(&["qualify-security-deep", "--run", "--verify=scan"]).is_err());
    assert!(parse_args(&["qualify-security-deep", "--begin", "--verify=scan"]).is_err());
    assert!(parse_args(&["qualify-security-deep", "--run", "--run"]).is_err());
    assert!(parse_args(&["qualify-security-deep", "--verify=one", "--verify=two"]).is_err());
    assert!(parse_args(&["build", "--begin"]).is_err());
    assert!(parse_args(&["build", "--verify=scan"]).is_err());
    assert!(parse_args(&["build", "--run"]).is_err());
}

#[test]
fn unsigned_external_result_directories_have_no_seal_action() {
    for task in ["qualify-mutants-grade", "qualify-security-deep"] {
        let error = parse_args(&[task, "--seal=unsigned-bundle"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown option `--seal=unsigned-bundle`"));
    }
}

#[test]
fn every_exceptional_task_is_named_in_the_not_in_ci_block() {
    let help = help();
    let block = help
        .split_once("NOT IN CI (exceptional dependency, interaction, or runtime):\n")
        .unwrap()
        .1
        .split_once("\nHOUSEKEEPING:")
        .unwrap()
        .0;
    let names: std::collections::BTreeSet<&str> = block
        .lines()
        .filter(|line| line.starts_with("    ") && !line.starts_with("                      "))
        .filter_map(|line| line.split_whitespace().next())
        .map(|name| name.split_once(' ').map_or(name, |(task, _)| task))
        .collect();
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "baseline",
            "debug",
            "model-check",
            "mutate",
            "qualify-advisory",
            "qualify-mutants-grade",
            "qualify-security-deep",
            "qualify-vmware",
            "vmware",
        ])
    );
}
