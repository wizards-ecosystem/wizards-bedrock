use std::fs::{File, OpenOptions};
use std::process::Command;
use std::time::{Duration, Instant};

use super::journal::{LAST_RUN, report_name};
use super::source::{
    MAX_GIT_IDENTITY_BYTES, MAX_TRACKED_FILE_BYTES, digest_file, digest_file_with,
    finish_staged_operation_with, git_text, parse_tracked_paths, require_confined_absolute_file,
    restore_file, write_source_with,
};
use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink};

const CHILD_STATE: &str = "WIZARD_MUTATION_CHILD_STATE";
const CHILD_MARKER: &str = "WIZARD_MUTATION_CHILD_MARKER";

struct TempTree(PathBuf);

impl TempTree {
    fn new(name: &str) -> Self {
        let path = crate::test_support::reserve_temp_directory(&format!("wizard-mutation-{name}"));
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("removing mutation test directory");
    }
}

fn fixture_journal() -> Journal {
    Journal {
        generation: 0,
        commit: "1".repeat(40),
        registry: "2".repeat(64),
        source_digest: "3".repeat(64),
        index_digest: "4".repeat(64),
        active: None,
        completed: Vec::new(),
    }
}

/// Identities that are neither positions nor contiguous, as retired and interleaved entries leave.
fn ids_for(journal: &Journal) -> Vec<u16> {
    (0..journal.completed.len())
        .map(|index| 17 + 2 * index as u16)
        .collect()
}

fn fixture_baseline(journal: &Journal) -> Baseline {
    Baseline {
        root: PathBuf::from("fixture-root"),
        commit: journal.commit.clone(),
        tracked: Vec::new(),
        source_digest: journal.source_digest.clone(),
        index_path: PathBuf::from("fixture-index"),
        index_digest: journal.index_digest.clone(),
    }
}

#[test]
fn git_path_order_is_not_component_order() {
    let listed = b"boot/limine.conf\0boot/limine/BOOTX64.EFI\0";
    assert_eq!(
        parse_tracked_paths(listed).unwrap(),
        [
            PathBuf::from("boot/limine.conf"),
            PathBuf::from("boot/limine/BOOTX64.EFI")
        ]
    );
}

#[test]
fn tracked_path_order_refuses_duplicates_and_reversals() {
    assert!(parse_tracked_paths(b"alpha\0alpha\0").is_err());
    assert!(parse_tracked_paths(b"bravo\0alpha\0").is_err());
}

#[test]
fn an_absolute_git_path_must_resolve_to_one_confined_direct_file() {
    let temp = TempTree::new("absolute-git-path");
    let root = temp.0.join("repository");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let index = root.join(".git").join("index");
    std::fs::write(&index, b"index").unwrap();
    let admitted =
        require_confined_absolute_file(&root, index.to_str().unwrap(), "fixture Git index")
            .unwrap();
    assert_eq!(
        std::fs::canonicalize(admitted).unwrap(),
        std::fs::canonicalize(&index).unwrap()
    );

    let error = require_confined_absolute_file(&root, ".git/index", "relative Git index")
        .expect_err("relative Git index was accepted");
    assert!(
        error
            .to_string()
            .contains("relative Git index is not an absolute path"),
        "{error:#}"
    );
    let same_name_inside = root.join("outside");
    std::fs::write(&same_name_inside, b"inside").unwrap();
    let outside = temp.0.join("outside");
    std::fs::write(&outside, b"outside").unwrap();
    assert!(
        require_confined_absolute_file(&root, outside.to_str().unwrap(), "outside Git index")
            .is_err()
    );
}

#[test]
fn a_report_names_each_outcome_by_its_permanent_mutation_identity() {
    let mut journal = fixture_journal();
    journal.completed = vec![Outcome::Caught, Outcome::Survived];
    let mut report = Report::from(&journal, &[17, 1_413]);
    report.sequence = 1;
    let encoded = report.encode_with_checksum();
    assert!(encoded.contains("\nid 17 result caught\nid 1413 result survived\n"));
    assert_eq!(Report::parse(&encoded).unwrap().ids, [17, 1_413]);

    let rechecked = |body: String| {
        let digest = sha256::hex(&sha256::digest(body.as_bytes()));
        Report::parse(&format!("{body}checksum {digest}\n"))
    };
    assert!(rechecked(report.body().replacen("id 17 ", "", 1)).is_err());
    assert!(rechecked(report.body().replacen("id 17 ", "id seventeen ", 1)).is_err());
    assert!(rechecked(report.body().replacen("id 17 ", "id 017 ", 1)).is_err());
}

#[test]
fn a_completed_report_binds_totals_and_verdict_to_every_outcome() {
    let mut pass_journal = fixture_journal();
    pass_journal.completed = vec![Outcome::Caught, Outcome::Caught];
    let mut pass = Report::from(&pass_journal, &ids_for(&pass_journal));
    pass.sequence = 1;
    assert_eq!(Report::parse(&pass.encode_with_checksum()).unwrap(), pass);
    assert_eq!((pass.caught, pass.survived, pass.inconclusive), (2, 0, 0));
    assert_eq!(pass.verdict, ReportVerdict::Pass);
    pass.require_pass(&fixture_baseline(&pass_journal), &pass_journal.registry, 2)
        .expect("current all-caught report");

    let mut failed_journal = fixture_journal();
    failed_journal.completed = vec![
        Outcome::Caught,
        Outcome::Survived,
        Outcome::Inconclusive("wrong machine verdict".to_string()),
    ];
    let mut failed = Report::from(&failed_journal, &ids_for(&failed_journal));
    failed.sequence = 2;
    assert_eq!(
        Report::parse(&failed.encode_with_checksum()).unwrap(),
        failed
    );
    assert_eq!(
        (failed.caught, failed.survived, failed.inconclusive),
        (1, 1, 1)
    );
    assert_eq!(failed.verdict, ReportVerdict::Fail);
    assert!(
        failed
            .require_pass(
                &fixture_baseline(&failed_journal),
                &failed_journal.registry,
                3
            )
            .is_err()
    );

    let mut contradictory = failed.clone();
    contradictory.verdict = ReportVerdict::Pass;
    assert!(Report::parse(&contradictory.encode_with_checksum()).is_err());
    let noncanonical_body = pass.body().replacen("sequence 1\n", "sequence 01\n", 1);
    let noncanonical = format!(
        "{noncanonical_body}checksum {}\n",
        sha256::hex(&sha256::digest(noncanonical_body.as_bytes()))
    );
    assert!(Report::parse(&noncanonical).is_err());
    let mut stale = pass.clone();
    stale.commit = "9".repeat(40);
    assert!(
        stale
            .require_pass(&fixture_baseline(&pass_journal), &pass_journal.registry, 2)
            .is_err()
    );
}

#[test]
fn a_new_failed_sweep_supersedes_but_does_not_erase_an_older_pass() {
    let temp = TempTree::new("report-history");
    let store = JournalStore::new(temp.0.clone()).unwrap();
    let mut journal = fixture_journal();
    journal.completed = vec![Outcome::Caught];
    let pass = store
        .save_report(Report::from(&journal, &ids_for(&journal)))
        .unwrap();
    journal.completed = vec![Outcome::Survived];
    let failed = store
        .save_report(Report::from(&journal, &ids_for(&journal)))
        .unwrap();
    store.write_last_run(&failed).unwrap();

    assert_eq!(pass.sequence, 1);
    assert_eq!(failed.sequence, 2);
    assert!(temp.0.join(report_name(1)).is_file());
    assert!(temp.0.join(report_name(2)).is_file());
    assert_eq!(
        store.latest_report().unwrap().unwrap().1.verdict,
        ReportVerdict::Fail
    );
    let last = std::fs::read_to_string(temp.0.join(LAST_RUN)).unwrap();
    assert_eq!(Report::parse(&last).unwrap(), failed);
}

#[test]
fn a_recorded_parent_pass_selects_the_newest_report_and_retains_the_sweep_lock() {
    let temp = TempTree::new("recorded-parent-pass");
    let paths = Paths::new(temp.0.join("repo"), Some(&temp.0.join("target")));
    let state_root = prepare_state_root(paths.target_dir()).unwrap();
    let mut journal = fixture_journal();
    let key = format!("{}-{}", &journal.commit[..12], &journal.registry[..12]);
    let state = state_root.join(key);
    std::fs::create_dir(&state).unwrap();
    let store = JournalStore::new(state).unwrap();
    journal.completed = vec![Outcome::Caught];
    let pass = store
        .save_report(Report::from(&journal, &ids_for(&journal)))
        .unwrap();
    let pass_path = store.directory.join(report_name(pass.sequence));
    let pass_bytes = read_state_text(&pass_path, "fixture mutation report").unwrap();
    let pass_digest = sha256::hex(&sha256::digest(pass_bytes.as_bytes()));

    let current = recorded_pass(
        &paths,
        &journal.commit,
        &journal.registry,
        &journal.source_digest,
        &journal.index_digest,
        1,
        &pass_digest,
    )
    .unwrap();
    assert!(acquire_lock(&state_root.join(LOCK_NAME)).is_err());
    drop(current);

    journal.completed = vec![Outcome::Survived];
    store
        .save_report(Report::from(&journal, &ids_for(&journal)))
        .unwrap();
    assert!(
        recorded_pass(
            &paths,
            &journal.commit,
            &journal.registry,
            &journal.source_digest,
            &journal.index_digest,
            1,
            &pass_digest,
        )
        .is_err()
    );
}

#[test]
fn a_current_pass_cannot_bind_bytes_from_a_substituted_report() {
    let mut journal = fixture_journal();
    journal.completed = vec![Outcome::Caught];
    let mut selected = Report::from(&journal, &ids_for(&journal));
    selected.sequence = 1;
    let selected_bytes = selected.encode_with_checksum();

    let mut substituted = selected.clone();
    substituted.sequence = 2;
    let substituted_bytes = substituted.encode_with_checksum();

    assert_eq!(
        bind_current_report(&selected, selected_bytes.as_bytes()).unwrap(),
        sha256::hex(&sha256::digest(selected_bytes.as_bytes()))
    );
    let error = bind_current_report(&selected, substituted_bytes.as_bytes()).unwrap_err();
    assert!(
        format!("{error:#}").contains("changed while selecting the current pass"),
        "{error:#}"
    );
}

#[test]
fn bounded_mutation_reader_refuses_a_final_component_link() {
    let temp = TempTree::new("bounded-reader-link");
    let target = temp.0.join("target");
    let link = temp.0.join("report");
    let payload = b"bounded report bytes";
    std::fs::write(&target, payload).unwrap();
    assert_eq!(read_bounded_bytes(&target, 64, "report").unwrap(), payload);
    create_file_symlink(&target, &link);
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let measured = std::cell::Cell::new(false);
    let result = read_bounded_bytes_with(&link, 64, "report", || {
        measured.set(true);
        Ok(())
    });
    assert!(
        result.is_err(),
        "the reader followed a final-component link"
    );
    assert!(
        !measured.get(),
        "a link was admitted before bounded reading"
    );
    assert_eq!(std::fs::read(&target).unwrap(), payload);
    std::fs::remove_file(link).unwrap();
}

#[test]
fn mutation_file_digest_refuses_a_final_component_link() {
    let temp = TempTree::new("file-digest-link");
    let target = temp.0.join("target");
    let link = temp.0.join("index");
    let payload = b"bounded index bytes";
    std::fs::write(&target, payload).unwrap();
    assert_eq!(
        digest_file(&target, 64, "index").unwrap(),
        sha256::hex(&sha256::digest(payload))
    );
    create_file_symlink(&target, &link);
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let measured = std::cell::Cell::new(false);
    let result = digest_file_with(&link, 64, "index", || {
        measured.set(true);
        Ok(())
    });
    assert!(
        result.is_err(),
        "the digest followed a final-component link"
    );
    assert!(
        !measured.get(),
        "a link was admitted before bounded hashing"
    );
    assert_eq!(std::fs::read(&target).unwrap(), payload);
    std::fs::remove_file(link).unwrap();
}

#[test]
fn oversized_mutation_state_is_refused_before_payload_allocation() {
    let temp = TempTree::new("oversized-state");
    let path = temp.0.join("oversized.report");
    File::create(&path)
        .unwrap()
        .set_len(MAX_STATE_FILE_BYTES + 1)
        .unwrap();
    let error = read_state_text(&path, "mutation report")
        .unwrap_err()
        .to_string();
    assert!(error.contains("exceeding the 16777216-byte allocation limit"));
}

#[test]
fn mutation_state_growth_is_refused_after_measurement() {
    let temp = TempTree::new("growing-state");
    let path = temp.0.join("growing.report");
    std::fs::write(&path, b"x").unwrap();
    let growing = path.clone();

    let error = read_bounded_bytes_with(&path, 4, "mutation report", move || {
        let mut options = OpenOptions::new();
        OpenOptions::write(&mut options, true);
        OpenOptions::open(&options, growing)?.set_len(5)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond the 4-byte allocation limit while being read"),
        "{error}"
    );
}

#[test]
fn mutation_digest_refuses_an_initial_file_beyond_its_ceiling() {
    let temp = TempTree::new("oversized-digest");
    let path = temp.0.join("oversized.index");
    File::create(&path).unwrap().set_len(5).unwrap();

    let error = digest_file(&path, 4, "Git index").unwrap_err().to_string();

    assert!(error.contains("exceeding its 4-byte limit"), "{error}");
}

#[test]
fn mutation_digest_refuses_growth_after_measurement() {
    let temp = TempTree::new("growing-digest");
    let path = temp.0.join("growing.index");
    std::fs::write(&path, b"x").unwrap();
    let growing = path.clone();

    let error = digest_file_with(&path, 4, "Git index", move || {
        let mut options = OpenOptions::new();
        OpenOptions::write(&mut options, true);
        OpenOptions::open(&options, growing)?.set_len(5)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond its 4-byte limit while being hashed"),
        "{error}"
    );
}

#[test]
fn mutation_digest_refuses_truncation_after_measurement() {
    let temp = TempTree::new("truncated-digest");
    let path = temp.0.join("truncated.index");
    std::fs::write(&path, b"1234").unwrap();
    let shrinking = path.clone();

    let error = digest_file_with(&path, 4, "Git index", move || {
        let mut options = OpenOptions::new();
        OpenOptions::write(&mut options, true);
        OpenOptions::open(&options, shrinking)?.set_len(3)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("changed length while being hashed"),
        "{error}"
    );
}

#[test]
fn oversized_tracked_sources_are_refused_before_streaming() {
    let temp = TempTree::new("oversized-tracked-source");
    let path = temp.0.join("oversized");
    File::create(&path)
        .unwrap()
        .set_len(MAX_TRACKED_FILE_BYTES + 1)
        .unwrap();

    let error = digest_tracked(&temp.0, &[PathBuf::from("oversized")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("exceeding its 16777216-byte limit"),
        "{error}"
    );
}

#[test]
fn tracked_source_trees_have_an_independent_total_size_bound() {
    let temp = TempTree::new("oversized-tracked-tree");
    let mut tracked = Vec::new();
    for index in 0..5 {
        let name = format!("input-{index}");
        File::create(temp.0.join(&name))
            .unwrap()
            .set_len(MAX_TRACKED_FILE_BYTES)
            .unwrap();
        tracked.push(PathBuf::from(name));
    }

    let error = digest_tracked(&temp.0, &tracked).unwrap_err().to_string();
    assert!(
        error.contains("exceeding its 67108864-byte limit"),
        "{error}"
    );
}

#[test]
fn streamed_tracked_source_digest_preserves_the_framed_schema() {
    let temp = TempTree::new("tracked-source-schema");
    std::fs::create_dir(temp.0.join("nested")).unwrap();
    std::fs::write(temp.0.join("alpha"), b"first").unwrap();
    std::fs::write(temp.0.join("nested/beta"), b"second").unwrap();
    let tracked = [PathBuf::from("alpha"), PathBuf::from("nested/beta")];
    let observed = digest_tracked(&temp.0, &tracked).unwrap();

    let mut legacy = Vec::new();
    for relative in &tracked {
        let name = relative.to_str().unwrap();
        let bytes = std::fs::read(temp.0.join(relative)).unwrap();
        legacy.extend_from_slice(&(name.len() as u64).to_le_bytes());
        legacy.extend_from_slice(name.as_bytes());
        legacy.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        legacy.extend_from_slice(&bytes);
    }
    assert_eq!(observed, sha256::hex(&sha256::digest(&legacy)));
}

fn git_test(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .expect("spawning fixture Git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const MUTATION_TARGET: &str = "libs/probe/src/lib.rs";
const NON_TARGET: &str = "libs/probe/src/other.rs";
const REGRESSION_ARTIFACT: &str = "libs/probe/tests/properties.proptest-regressions";

fn mutation_session(name: &str) -> (TempTree, Paths) {
    let temp = TempTree::new(name);
    let repository = temp.0.join("repository");
    let target = temp.0.join("target");
    std::fs::create_dir_all(repository.join("libs/probe/src"))
        .expect("repository source directory");
    git_test(&repository, &["init", "--quiet"]);
    std::fs::write(repository.join(MUTATION_TARGET), b"target source").expect("target source");
    std::fs::write(repository.join(NON_TARGET), b"other source").expect("non-target source");
    git_test(&repository, &["add", "."]);
    git_test(
        &repository,
        &[
            "-c",
            "user.name=Wizard Test",
            "-c",
            "user.email=wizard@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    (temp, Paths::new(repository, Some(&target)))
}

#[test]
fn initial_copy_refuses_a_nested_redirect_before_creating_outside_paths() {
    let tree = TempTree::new("nested-copy-redirect");
    let source = tree.0.join("source");
    let destination = tree.0.join("destination");
    let outside = tree.0.join("outside");
    std::fs::create_dir_all(source.join("nested/deeper")).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(source.join("nested/deeper/tracked"), b"tracked source").unwrap();
    std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
    let redirect = destination.join("nested");
    create_directory_symlink(&outside, &redirect);

    assert!(
        copy_tracked(
            &source,
            &destination,
            &[PathBuf::from("nested/deeper/tracked")],
        )
        .is_err()
    );
    assert!(
        !outside.join("deeper").exists(),
        "recursive parent creation escaped the mutation copy root"
    );
    assert_eq!(
        std::fs::read(outside.join("sentinel")).unwrap(),
        b"outside authority"
    );
    crate::test_support::remove_directory_link(&redirect);
}

#[test]
fn restore_refuses_same_byte_external_state_before_its_fast_path() {
    let tree = TempTree::new("nested-restore-redirect");
    let snapshot = tree.0.join("snapshot");
    let workspace = tree.0.join("workspace");
    let outside = tree.0.join("outside");
    std::fs::create_dir_all(snapshot.join("nested/deeper")).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir_all(outside.join("deeper")).unwrap();
    std::fs::write(snapshot.join("nested/deeper/tracked"), b"same bytes").unwrap();
    std::fs::write(outside.join("deeper/tracked"), b"same bytes").unwrap();
    let redirect = workspace.join("nested");
    create_directory_symlink(&outside, &redirect);

    assert!(restore_file(&snapshot, &workspace, "nested/deeper/tracked").is_err());
    assert_eq!(
        std::fs::read(outside.join("deeper/tracked")).unwrap(),
        b"same bytes"
    );
    crate::test_support::remove_directory_link(&redirect);
}

fn contaminate(workspace: &Path) {
    std::fs::write(workspace.join(MUTATION_TARGET), b"mutated target").expect("mutating target");
    std::fs::write(workspace.join(NON_TARGET), b"mutated non-target").expect("mutating non-target");
    let regression = workspace.join(REGRESSION_ARTIFACT);
    std::fs::create_dir_all(regression.parent().expect("regression parent"))
        .expect("regression directory");
    std::fs::write(regression, b"cc generated regression seed").expect("regression artifact");
}

fn assert_pristine(workspace: &Path) {
    assert_eq!(
        std::fs::read(workspace.join(MUTATION_TARGET)).unwrap(),
        b"target source"
    );
    assert_eq!(
        std::fs::read(workspace.join(NON_TARGET)).unwrap(),
        b"other source"
    );
    assert!(!workspace.join(REGRESSION_ARTIFACT).exists());
}

#[test]
fn a_finished_mutation_restores_the_whole_sandbox() {
    let (_temp, paths) = mutation_session("whole-sandbox");
    let mut session = Session::open(&paths, &"a".repeat(64), 1).expect("isolated session");
    let workspace = session.paths().root().to_path_buf();
    session.begin(0).expect("starting mutation");
    contaminate(&workspace);

    session
        .finish(0, MUTATION_TARGET, Outcome::Caught)
        .expect("finishing mutation");
    assert_pristine(&workspace);

    contaminate(&workspace);
    assert_eq!(session.complete(&[1]).unwrap(), [Outcome::Caught]);
    assert_pristine(&workspace);
    let evidence = current_pass(&paths, &"a".repeat(64), 1).expect("current pass evidence");
    assert_eq!(evidence.evidence().mutation_count, 1);
    assert!(evidence.evidence().report_path.is_file());
}

#[test]
fn a_current_pass_blocks_a_newer_sweep_until_consumed() {
    let (_temp, paths) = mutation_session("current-pass-lock");
    let registry = "d".repeat(64);
    Session::open(&paths, &registry, 0)
        .expect("isolated session")
        .complete(&[])
        .expect("completed report");
    let pass = current_pass(&paths, &registry, 0).expect("current pass");

    let Err(error) = Session::open(&paths, &registry, 0) else {
        panic!("a newer sweep acquired the current pass's lock");
    };
    assert!(
        format!("{error:#}").contains("another mutation sweep holds"),
        "{error:#}"
    );

    drop(pass);
    Session::open(&paths, &registry, 0).expect("lock released after pass consumer");
}

#[test]
fn a_redirected_mutation_state_ancestor_is_refused_before_cleanup() {
    let (temp, paths) = mutation_session("redirected-state");
    let registry = "e".repeat(64);
    let commit = git_text(
        paths.root(),
        ["rev-parse", "--verify", "HEAD"],
        MAX_GIT_IDENTITY_BYTES,
        "fixture commit",
    )
    .expect("fixture commit");
    let key = format!("{}-{}", &commit[..12], &registry[..12]);
    let state_root = paths.target_dir().join(STATE_ROOT_NAME);
    let outside = temp.0.join("outside-state");
    let outside_workspace = outside.join("workspace");
    std::fs::create_dir_all(&state_root).expect("mutation state root");
    std::fs::create_dir_all(&outside_workspace).expect("outside workspace");
    let sentinel = outside_workspace.join("keep");
    std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
    create_directory_symlink(&outside, &state_root.join(key));

    let Err(error) = Session::open(&paths, &registry, 0) else {
        panic!("redirected mutation state was accepted");
    };
    assert!(
        format!("{error:#}").contains("mutation state is not a real directory"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("outside sentinel survives"),
        b"outside authority"
    );
}

#[test]
fn recursive_cleanup_revalidates_the_whole_mutation_state_ancestry() {
    let temp = TempTree::new("redirected-cleanup");
    let target = temp.0.join("target");
    let state_root = target.join(STATE_ROOT_NAME);
    let state = state_root.join("111111111111-222222222222");
    let outside = temp.0.join("outside-cleanup");
    let source = outside.join("source");
    let workspace = outside.join("workspace");
    std::fs::create_dir_all(&state_root).expect("mutation state root");
    std::fs::create_dir_all(&source).expect("outside source");
    std::fs::create_dir_all(&workspace).expect("outside workspace");
    let sentinel = workspace.join("keep");
    std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
    create_directory_symlink(&outside, &state);

    let error = restore_workspace(&state.join("source"), &state.join("workspace"), &[])
        .expect_err("redirected cleanup was accepted");
    assert!(
        format!("{error:#}").contains("mutation state is not a real directory"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("outside sentinel survives"),
        b"outside authority"
    );
}

#[test]
fn tracked_snapshot_and_workspace_redirects_are_refused_at_use() {
    let (temp, paths) = mutation_session("redirected-tracked-paths");
    let registry = "f".repeat(64);
    let session = Session::open(&paths, &registry, 0).expect("isolated session");
    let outside_snapshot = temp.0.join("outside-snapshot");
    std::fs::create_dir(&outside_snapshot).expect("outside snapshot directory");
    let outside_snapshot_file = outside_snapshot.join("lib.rs");
    std::fs::write(&outside_snapshot_file, b"outside snapshot authority")
        .expect("outside snapshot file");
    let snapshot_parent = session.snapshot.join("libs/probe/src");
    std::fs::remove_dir_all(&snapshot_parent).expect("removing copied snapshot parent");
    create_directory_symlink(&outside_snapshot, &snapshot_parent);

    let error = session
        .original(MUTATION_TARGET)
        .expect_err("redirected snapshot read was accepted");
    assert!(
        format!("{error:#}").contains("mutation source file is not a real directory"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(&outside_snapshot_file).expect("outside snapshot survives"),
        b"outside snapshot authority"
    );
    drop(session);

    let (_temp, paths) = mutation_session("redirected-workspace-parent");
    let session = Session::open(&paths, &registry, 0).expect("isolated session");
    let outside = paths.root().parent().unwrap().join("outside-workspace");
    std::fs::create_dir(&outside).expect("outside workspace directory");
    let sentinel = outside.join("lib.rs");
    std::fs::write(&sentinel, b"outside workspace authority").expect("outside sentinel");
    let workspace_parent = session.paths().root().join("libs/probe/src");
    std::fs::remove_dir_all(&workspace_parent).expect("removing copied workspace parent");
    create_directory_symlink(&outside, &workspace_parent);

    let error = Session::write(&session, MUTATION_TARGET, "redirected mutation")
        .expect_err("redirected workspace write was accepted");
    assert!(
        format!("{error:#}").contains("mutation workspace file is not a real directory"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("outside workspace survives"),
        b"outside workspace authority"
    );

    crate::test_support::remove_directory_link(&workspace_parent);
    std::fs::create_dir(&workspace_parent).expect("recreating workspace parent");
    let outside_file = outside.join("direct-file");
    std::fs::write(&outside_file, b"outside file authority").expect("outside file");
    create_file_symlink(&outside_file, &workspace_parent.join("lib.rs"));
    let error = Session::write(&session, MUTATION_TARGET, "redirected mutation")
        .expect_err("redirected workspace file was accepted");
    assert!(
        format!("{error:#}").contains("mutation workspace file is not a real file"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(&outside_file).expect("outside file survives"),
        b"outside file authority"
    );
}

#[test]
fn workspace_replacement_never_writes_through_an_external_hardlink() {
    let temp = TempTree::new("workspace-hardlink");
    let workspace = temp.0.join("workspace");
    let outside = temp.0.join("outside");
    std::fs::create_dir(&workspace).expect("workspace directory");
    std::fs::create_dir(&outside).expect("outside directory");
    let sentinel = outside.join("authority");
    let destination = workspace.join("tracked");
    std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
    std::fs::hard_link(&sentinel, &destination).expect("workspace hard link");

    write_source(&destination, b"isolated mutation").expect("replacing workspace source");

    assert_eq!(
        std::fs::read(&sentinel).expect("outside sentinel survives"),
        b"outside authority"
    );
    assert_eq!(
        std::fs::read(&destination).expect("replacement source"),
        b"isolated mutation"
    );

    let (temp, paths) = mutation_session("session-hardlink");
    let session = Session::open(&paths, &"7".repeat(64), 0).expect("isolated session");
    let destination = session.paths().root().join(MUTATION_TARGET);
    let sentinel = temp.0.join("session-authority");
    std::fs::write(&sentinel, b"target source").expect("session sentinel");
    std::fs::remove_file(&destination).expect("unlinking workspace source");
    std::fs::hard_link(&sentinel, &destination).expect("session workspace hard link");

    Session::write(&session, MUTATION_TARGET, "isolated session mutation")
        .expect("session replacement");

    assert_eq!(
        std::fs::read(&sentinel).expect("session sentinel survives"),
        b"target source"
    );
    assert_eq!(
        std::fs::read(&destination).expect("session replacement source"),
        b"isolated session mutation"
    );
}

#[test]
fn a_failed_source_publication_restores_the_previous_authority() {
    let temp = TempTree::new("source-publication-rollback");
    let destination = temp.0.join("tracked.rs");
    std::fs::write(&destination, b"complete source").expect("complete source");

    let error = write_source_with(&destination, b"partial mutation", |_, _| {
        Err(std::io::Error::other("injected publication refusal"))
    })
    .expect_err("failed publication was accepted");

    assert!(format!("{error:#}").contains("injected publication refusal"));
    assert_eq!(
        std::fs::read(&destination).expect("restored source"),
        b"complete source"
    );
    assert_eq!(crate::paths::directory_entries(&temp.0).unwrap().len(), 1);
}

#[test]
fn failed_stage_cleanup_reports_both_operation_and_removal_failures() {
    let temp = TempTree::new("stage-cleanup-removal");
    let staging = temp.0.join("stage");
    std::fs::write(&staging, b"partial bytes").expect("staging file");

    let error = finish_staged_operation_with(
        &temp.0,
        &staging,
        "fixture stage",
        Err::<(), _>(anyhow::anyhow!("injected operation failure")),
        |_| Err(std::io::Error::other("injected cleanup refusal")),
    )
    .expect_err("cleanup failure was discarded");
    let error = format!("{error:#}");

    assert!(error.contains("injected operation failure"), "{error}");
    assert!(error.contains("injected cleanup refusal"), "{error}");
    assert!(staging.exists(), "the injected remover unexpectedly ran");
}

#[test]
fn failed_stage_cleanup_rejects_a_hostile_stage_replacement() {
    let temp = TempTree::new("stage-cleanup-replacement");
    let staging = temp.0.join("stage");
    std::fs::create_dir(&staging).expect("hostile staging directory");

    let error = finish_staged_operation_with(
        &temp.0,
        &staging,
        "fixture stage",
        Err::<(), _>(anyhow::anyhow!("injected operation failure")),
        |_| panic!("a hostile stage must not reach removal"),
    )
    .expect_err("hostile stage replacement was ignored");
    let error = format!("{error:#}");

    assert!(error.contains("injected operation failure"), "{error}");
    assert!(
        error.contains("fixture stage is not a real file"),
        "{error}"
    );
}

#[test]
fn a_mutation_lock_never_writes_through_an_external_hardlink() {
    let temp = TempTree::new("lock-hardlink");
    let state = temp.0.join("state");
    let outside = temp.0.join("outside");
    std::fs::create_dir(&state).expect("mutation state directory");
    std::fs::create_dir(&outside).expect("outside directory");
    let sentinel = outside.join("authority");
    std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
    std::fs::hard_link(&sentinel, state.join(LOCK_NAME)).expect("lock hard link");

    let lock = acquire_lock(&state.join(LOCK_NAME)).expect("hard-linked lock is byte-safe");
    drop(lock);

    assert_eq!(
        std::fs::read(&sentinel).expect("outside sentinel survives"),
        b"outside authority"
    );
}

#[test]
fn journal_paths_cannot_be_substituted_after_the_store_opens() {
    let temp = TempTree::new("redirected-journal");
    let state = temp.0.join("state");
    std::fs::create_dir(&state).expect("journal state directory");
    let store = JournalStore::new(state.clone()).expect("journal store");
    let outside = temp.0.join("outside-journal");
    std::fs::create_dir(&outside).expect("outside journal directory");
    let outside_slot = outside.join("journal");
    let journal = fixture_journal();
    std::fs::write(&outside_slot, journal.encode_with_checksum()).expect("outside journal");
    create_file_symlink(&outside_slot, &store.slot(1));

    let error = store
        .load()
        .expect_err("redirected journal read was accepted");
    assert!(
        format!("{error:#}").contains("mutation journal slot is not a real file"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read_to_string(&outside_slot).expect("outside journal survives"),
        journal.encode_with_checksum()
    );

    std::fs::remove_file(store.slot(1)).expect("removing journal redirect");
    let outside_last_run = outside.join("last-run");
    std::fs::write(&outside_last_run, b"outside report authority").expect("outside report");
    std::fs::hard_link(&outside_last_run, state.join(LAST_RUN)).expect("hard-linking report");
    let mut report = Report::from(&journal, &ids_for(&journal));
    report.sequence = 1;
    store
        .write_last_run(&report)
        .expect("publishing last report");
    assert_eq!(
        std::fs::read(&outside_last_run).expect("outside report survives"),
        b"outside report authority"
    );
    assert_eq!(
        Report::parse(&std::fs::read_to_string(state.join(LAST_RUN)).unwrap()).unwrap(),
        report
    );

    let redirected = temp.0.join("redirected-state");
    std::fs::rename(&state, &redirected).expect("moving journal state");
    create_directory_symlink(&redirected, &state);
    let error = store
        .save(&mut fixture_journal())
        .expect_err("redirected journal directory was accepted");
    assert!(
        format!("{error:#}").contains("mutation journal directory is not a real directory"),
        "{error:#}"
    );
    assert!(!redirected.join("journal.1").exists());
}

#[test]
fn an_interrupted_session_is_sanitized_before_recovery() {
    let (_temp, paths) = mutation_session("recovery-sandbox");
    let mut session = Session::open(&paths, &"b".repeat(64), 1).expect("isolated session");
    let workspace = session.paths().root().to_path_buf();
    session.begin(0).expect("starting mutation");
    drop(session);
    contaminate(&workspace);

    let mut resumed = Session::open(&paths, &"b".repeat(64), 1).expect("resuming session");
    assert_eq!(resumed.active(), Some(0));
    assert_pristine(&workspace);

    contaminate(&workspace);
    resumed
        .recover(0, MUTATION_TARGET)
        .expect("recovering mutation");
    assert_eq!(resumed.active(), None);
    assert_pristine(&workspace);

    resumed.begin(0).expect("restarting mutation");
    Session::write(&resumed, MUTATION_TARGET, "mutated target").expect("applying mutation");
    resumed
        .finish(0, MUTATION_TARGET, Outcome::Caught)
        .expect("finishing mutation");
    assert_eq!(resumed.complete(&[1]).unwrap(), [Outcome::Caught]);
}

#[test]
fn a_session_builds_only_from_its_tracked_copy() {
    let temp = TempTree::new("session");
    let repository = temp.0.join("repository");
    let target = temp.0.join("target");
    std::fs::create_dir(&repository).expect("repository directory");
    git_test(&repository, &["init", "--quiet"]);
    std::fs::write(repository.join(".gitignore"), "ignored\n").expect("ignore file");
    std::fs::write(repository.join("tracked"), b"source bytes").expect("tracked file");
    std::fs::write(repository.join("ignored"), b"not an input").expect("ignored file");
    git_test(&repository, &["add", ".gitignore", "tracked"]);
    git_test(
        &repository,
        &[
            "-c",
            "user.name=Wizard Test",
            "-c",
            "user.email=wizard@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    let source_before = std::fs::read(repository.join("tracked")).expect("source before");
    let index_before = std::fs::read(repository.join(".git/index")).expect("index before");

    let paths = Paths::new(repository.clone(), Some(&target));
    let session = Session::open(&paths, &"a".repeat(64), 0).expect("isolated session");
    let workspace = session.paths().root().to_path_buf();
    assert_ne!(workspace, repository);
    assert_eq!(
        std::fs::read(workspace.join("tracked")).unwrap(),
        source_before
    );
    assert!(!workspace.join("ignored").exists());
    assert!(!workspace.join(".git").exists());
    assert_eq!(session.complete(&[]).unwrap(), []);

    assert_eq!(
        std::fs::read(repository.join("tracked")).unwrap(),
        source_before
    );
    assert_eq!(
        std::fs::read(repository.join(".git/index")).unwrap(),
        index_before
    );
    let status = Command::new("git")
        .current_dir(&repository)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .expect("final Git status");
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

#[test]
fn a_changed_snapshot_cannot_authorize_a_report_for_pristine_source() {
    let (_temp, paths) = mutation_session("changed-snapshot");
    let session = Session::open(&paths, &"c".repeat(64), 0).expect("isolated session");
    std::fs::write(
        session.snapshot.join(MUTATION_TARGET),
        b"substituted source",
    )
    .expect("changing copied snapshot");

    let error = session.complete(&[]).unwrap_err().to_string();
    assert!(error.contains("source snapshot changed"), "{error}");
    assert!(current_pass(&paths, &"c".repeat(64), 0).is_err());
}

fn chain_source(value: u8) -> String {
    format!("#[inline(always)]\npub fn value() -> u8 {{ {value} }}\n")
}

/// A three-crate inlining chain whose old snapshot timestamp must not hide a restore from Cargo.
fn chain(name: &str) -> (TempTree, PathBuf, Paths) {
    let temp = TempTree::new(name);
    let snapshot = temp.0.join("snapshot");
    let workspace = temp.0.join("workspace");
    let build = temp.0.join("build");
    std::fs::create_dir_all(snapshot.join("libs/probe/src")).expect("snapshot source");
    for directory in ["libs/probe/src", "middle/src", "app/src"] {
        std::fs::create_dir_all(workspace.join(directory)).expect("workspace source");
    }
    for (path, contents) in [
        (
            "Cargo.toml",
            "[workspace]\nresolver = \"3\"\nmembers = [\"libs/probe\", \"middle\", \"app\"]\n",
        ),
        (
            "libs/probe/Cargo.toml",
            "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        ),
        (
            "middle/Cargo.toml",
            "[package]\nname = \"middle\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 [dependencies]\nprobe = { path = \"../libs/probe\" }\n",
        ),
        (
            "app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 [dependencies]\nmiddle = { path = \"../middle\" }\n",
        ),
        (
            "middle/src/lib.rs",
            "#[inline(always)]\npub fn seen() -> u8 { probe::value() }\n",
        ),
        (
            "app/src/main.rs",
            "fn main() { print!(\"{}\", middle::seen()); }\n",
        ),
    ] {
        std::fs::write(workspace.join(path), contents).expect("fixture file");
    }
    std::fs::write(snapshot.join(CHAIN_INPUT), chain_source(1)).expect("snapshot input");
    std::fs::write(workspace.join(CHAIN_INPUT), chain_source(1)).expect("pristine input");
    (temp, snapshot, Paths::new(workspace, Some(&build)))
}

const CHAIN_INPUT: &str = "libs/probe/src/lib.rs";

fn observed(paths: &Paths) -> String {
    let output = Command::new(env!("CARGO"))
        .current_dir(paths.root())
        .env(
            "CARGO_TARGET_DIR",
            crate::cargo::child_process_path(paths.target_dir()),
        )
        .args(["run", "--quiet", "--package", "app"])
        .output()
        .expect("running the fixture application");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("fixture output")
}

#[test]
fn a_restored_input_is_what_the_next_build_observes() {
    let (_temp, snapshot, paths) = chain("restore");
    assert_eq!(observed(&paths), "1");
    std::fs::write(paths.root().join(CHAIN_INPUT), chain_source(2)).expect("mutated input");
    assert_eq!(observed(&paths), "2");

    restore_file(&snapshot, paths.root(), CHAIN_INPUT).expect("restoring input");
    assert_eq!(
        std::fs::read_to_string(paths.root().join(CHAIN_INPUT)).unwrap(),
        chain_source(1)
    );
    assert_eq!(observed(&paths), "1");
}

/// A new sweep must not inherit the last sweep's final mutation from its retained build directory.
#[test]
fn a_recopied_workspace_is_what_the_next_build_observes() {
    let (_temp, snapshot, paths) = chain("recopy");
    assert_eq!(observed(&paths), "1");
    std::fs::write(paths.root().join(CHAIN_INPUT), chain_source(2)).expect("mutated input");
    assert_eq!(observed(&paths), "2");

    copy_tracked(&snapshot, paths.root(), &[PathBuf::from(CHAIN_INPUT)])
        .expect("recopying tracked source");
    assert_eq!(observed(&paths), "1");
}

/// The helper a parent test kills while a mutation is active; ignored unless that parent selects it.
#[test]
#[ignore]
fn interruption_child() {
    let state = PathBuf::from(std::env::var_os(CHILD_STATE).expect("child state path"));
    let marker = PathBuf::from(std::env::var_os(CHILD_MARKER).expect("child marker path"));
    let _lock = acquire_lock(&state.join(LOCK_NAME)).expect("child holds sweep lock");
    std::fs::write(state.join("workspace/tracked"), b"mutated")
        .expect("child applies isolated mutation");
    std::fs::write(marker, b"entered").expect("child announces active work");
    loop {
        std::thread::park();
    }
}

fn kill_at(state: &Path, marker: &Path) {
    let mut child = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "mutate::workspace::tests::interruption_child",
            "--ignored",
        ])
        .env(CHILD_STATE, state)
        .env(CHILD_MARKER, marker)
        .spawn()
        .expect("spawning interruptible mutation child");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(marker.exists(), "mutation child never entered its activity");
    let second_acquired = acquire_lock(&state.join(LOCK_NAME)).is_ok();
    child.kill().expect("killing mutation child");
    child.wait().expect("reaping mutation child");
    assert!(!second_acquired, "a live second sweep acquired the lock");
}

/// The released lock and active journal let the next process restore and rerun the killed mutation.
#[test]
fn a_killed_build_or_boot_resumes_outside_the_user_tree() {
    let temp = TempTree::new("interrupt");
    let state = temp.0.join("state");
    let source = state.join("source");
    let workspace = state.join("workspace");
    let user = temp.0.join("user");
    std::fs::create_dir_all(&source).expect("source directory");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    std::fs::create_dir_all(&user).expect("user directory");
    std::fs::write(source.join("tracked"), b"original").expect("snapshot file");
    std::fs::write(workspace.join("tracked"), b"original").expect("workspace file");
    std::fs::write(user.join("tracked"), b"user source").expect("user source");
    std::fs::write(user.join("index"), b"user index").expect("user index");
    let before_source = std::fs::read(user.join("tracked")).expect("source snapshot");
    let before_index = std::fs::read(user.join("index")).expect("index snapshot");
    let store = JournalStore::new(state.clone()).unwrap();
    let mut journal = fixture_journal();
    store.save(&mut journal).expect("initial journal");

    for activity in ["build", "boot"] {
        journal.active = Some(0);
        store.save(&mut journal).expect("active journal");
        let marker = state.join(format!("{activity}-entered"));
        kill_at(&state, &marker);

        let _lock = acquire_lock(&state.join(LOCK_NAME)).expect("released lock is reusable");
        let loaded = store
            .load()
            .expect("loading interrupted journal")
            .expect("journal remains");
        assert_eq!(loaded.active, Some(0));
        restore_file(&source, &workspace, "tracked").expect("resuming restores sandbox");
        assert_eq!(
            std::fs::read(workspace.join("tracked")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(user.join("tracked")).unwrap(), before_source);
        assert_eq!(std::fs::read(user.join("index")).unwrap(), before_index);
        journal = loaded;
        journal.active = None;
        store.save(&mut journal).expect("recovered journal");
    }
}

#[test]
fn journal_uses_the_newest_complete_slot_and_falls_back_from_a_torn_write() {
    let temp = TempTree::new("journal");
    let store = JournalStore::new(temp.0.clone()).unwrap();
    let mut journal = fixture_journal();
    store.save(&mut journal).expect("first generation");
    let older = journal.clone();
    journal.active = Some(0);
    store.save(&mut journal).expect("second generation");

    assert_eq!(store.load().unwrap(), Some(journal.clone()));
    std::fs::write(store.slot(journal.generation), b"torn").expect("tearing newest slot");
    assert_eq!(store.load().unwrap(), Some(older));
}
