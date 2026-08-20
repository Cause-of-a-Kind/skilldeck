use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use predicates::prelude::*;
use serde::Serialize;
use tempfile::TempDir;

fn bin() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin("skilldeck").unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

fn file_url(path: &Path) -> String {
    let mut path = path.display().to_string().replace('\\', "/");
    if !path.starts_with('/') {
        path = format!("/{path}");
    }
    format!("file://{path}")
}

fn commit_repo(dir: &Path) {
    Command::new("git")
        .arg("init")
        .arg("--initial-branch=master")
        .arg(dir)
        .status()
        .unwrap();
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "commit",
            "-m",
            "init",
        ],
    );
}

#[derive(Serialize)]
struct TestExternalSkills {
    skills: BTreeMap<String, TestExternalSkill>,
}

#[derive(Serialize)]
struct TestExternalSkill {
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subdirectory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "ref")]
    reference: Option<String>,
}

#[derive(Serialize)]
struct TestSkillGroups {
    groups: BTreeMap<String, TestSkillGroup>,
}

#[derive(Serialize)]
struct TestSkillGroup {
    skills: String,
}

fn write_external_skills(catalog: &Path, skills: Vec<(&str, TestExternalSkill)>) {
    let skills = skills
        .into_iter()
        .map(|(name, skill)| (name.to_string(), skill))
        .collect();
    fs::write(
        catalog.join("external-skills.toml"),
        toml::to_string(&TestExternalSkills { skills }).unwrap(),
    )
    .unwrap();
}

fn write_skill_groups(catalog: &Path, groups: Vec<(&str, &str)>) {
    let groups = groups
        .into_iter()
        .map(|(name, skills)| {
            (
                name.to_string(),
                TestSkillGroup {
                    skills: skills.to_string(),
                },
            )
        })
        .collect();
    fs::write(
        catalog.join("skill-groups.toml"),
        toml::to_string(&TestSkillGroups { groups }).unwrap(),
    )
    .unwrap();
}

fn no_external_skills(catalog: &Path) {
    fs::write(catalog.join("external-skills.toml"), "").unwrap();
}

fn no_skill_groups(catalog: &Path) {
    fs::write(catalog.join("skill-groups.toml"), "").unwrap();
}

fn make_catalog(base: &Path, dirname: &str, skill: &str, body: &str) -> std::path::PathBuf {
    let catalog = base.join(dirname);
    fs::create_dir_all(catalog.join("skills").join(skill)).unwrap();
    fs::write(catalog.join("skills").join(skill).join("SKILL.md"), body).unwrap();
    no_external_skills(&catalog);
    no_skill_groups(&catalog);
    commit_repo(&catalog);
    catalog
}

struct Fixture {
    tmp: TempDir,
    catalog: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let external = tmp.path().join("external");
        fs::create_dir_all(external.join("nested/ext-skill")).unwrap();
        fs::write(external.join("nested/ext-skill/SKILL.md"), "external v1").unwrap();
        fs::write(external.join("nested/ext-skill/extra.txt"), "extra").unwrap();
        fs::create_dir_all(external.join(".git")).unwrap();
        commit_repo(&external);

        let catalog = tmp.path().join("catalog");
        fs::create_dir_all(catalog.join("skills/alpha/deep")).unwrap();
        fs::write(catalog.join("skills/alpha/SKILL.md"), "alpha v1").unwrap();
        fs::write(catalog.join("skills/alpha/deep/file.txt"), "deep").unwrap();
        fs::create_dir_all(catalog.join("skills/beta")).unwrap();
        fs::write(catalog.join("skills/beta/SKILL.md"), "beta v1").unwrap();
        write_external_skills(
            &catalog,
            vec![(
                "ext",
                TestExternalSkill {
                    source: external.display().to_string(),
                    subdirectory: Some("nested/ext-skill".into()),
                    reference: Some("master".into()),
                },
            )],
        );
        write_skill_groups(&catalog, vec![("web", "alpha ext")]);
        commit_repo(&catalog);
        Self { tmp, catalog }
    }
    fn cmd(&self) -> assert_cmd::Command {
        let mut c = bin();
        c.env("SKILLDECK_CONFIG_DIR", self.tmp.path().join("cfg"));
        c.env("SKILLDECK_CATALOG_REPOSITORY", &self.catalog);
        c.env("SKILLDECK_CATALOG_REF", "master");
        c
    }
}

#[test]
fn help_and_version_commands_work() {
    bin()
        .arg("help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
    bin()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn init_refuses_to_replace_existing_config_without_force() {
    let f = Fixture::new();
    let cfg = f.tmp.path().join("cfg-init-force");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--repository"])
        .arg(&f.catalog)
        .args(["--reference", "master"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--repository"])
        .arg(&f.catalog)
        .args(["--reference", "master"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--force", "--repository"])
        .arg(&f.catalog)
        .args(["--reference", "master"])
        .assert()
        .success();
}

#[test]
fn init_writes_config_and_install_uses_env_precedence() {
    let f = Fixture::new();
    let cfg = f.tmp.path().join("cfg");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--repository"])
        .arg(&f.catalog)
        .args(["--reference", "master"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configured skilldeck"));
    let root = f.tmp.path().join("skills");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install", "--yes", "alpha"])
        .arg(&root)
        .assert()
        .success();
    assert!(root.join("alpha/SKILL.md").is_file());
}

#[test]
fn catalog_precedence_is_cli_then_env_then_global_config() {
    let tmp = TempDir::new().unwrap();
    let cfg_dir = tmp.path().join("cfg");
    let config_catalog = make_catalog(tmp.path(), "config-catalog", "same", "from config");
    let env_catalog = make_catalog(tmp.path(), "env-catalog", "same", "from env");
    let cli_catalog = make_catalog(tmp.path(), "cli-catalog", "same", "from cli");

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg_dir)
        .args(["init", "--yes", "--repository"])
        .arg(&config_catalog)
        .args(["--reference", "master"])
        .assert()
        .success();

    let env_root = tmp.path().join("env-root");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg_dir)
        .env("SKILLDECK_CATALOG_REPOSITORY", &env_catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--yes", "same"])
        .arg(&env_root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(env_root.join("same/SKILL.md")).unwrap(),
        "from env"
    );

    let cli_root = tmp.path().join("cli-root");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg_dir)
        .env("SKILLDECK_CATALOG_REPOSITORY", &env_catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--catalog-repository"])
        .arg(&cli_catalog)
        .args(["--catalog-ref", "master", "--yes", "same"])
        .arg(&cli_root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(cli_root.join("same/SKILL.md")).unwrap(),
        "from cli"
    );
}

#[test]
fn installs_first_party_external_group_and_manifest() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    f.cmd()
        .args(["install", "--yes", "alpha"])
        .arg(&root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(root.join("alpha/deep/file.txt")).unwrap(),
        "deep"
    );
    f.cmd()
        .args(["install", "ext"])
        .arg(&root)
        .assert()
        .success();
    assert!(root.join("ext/extra.txt").is_file());
    assert!(!root.join("ext/.git").exists());
    assert!(root.join(".skilldeck/installations.toml").is_file());
    let group_root = f.tmp.path().join("group");
    f.cmd()
        .args(["install-group", "--yes", "web"])
        .arg(&group_root)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Group install complete: 2 installed",
        ));
}

#[test]
fn force_and_prompt_safeguards() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    f.cmd()
        .args(["install", "alpha"])
        .arg(&root)
        .write_stdin("n\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
    f.cmd()
        .args(["install", "--yes", "alpha"])
        .arg(&root)
        .assert()
        .success();
    f.cmd()
        .args(["install", "alpha"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    f.cmd()
        .args(["install", "--force", "alpha"])
        .arg(&root)
        .assert()
        .success();
}

#[test]
fn typo_suggestions_and_validation() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    f.cmd()
        .args(["install", "--yes", "alpah"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Did you mean `alpha`"));
    assert!(
        !root.exists(),
        "typo should not create missing install root"
    );
    f.cmd()
        .args(["install", "--yes", "../bad"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid skill name"));
    f.cmd()
        .args(["install-group", "--yes", "wbe"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Did you mean `web`"));
}

#[test]
fn force_install_failure_preserves_existing_skill() {
    let f = Fixture::new();
    write_external_skills(
        &f.catalog,
        vec![(
            "bad",
            TestExternalSkill {
                source: f.catalog.display().to_string(),
                subdirectory: Some("does-not-exist".into()),
                reference: Some("master".into()),
            },
        )],
    );
    git(&f.catalog, &["add", "."]);
    git(
        &f.catalog,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "commit",
            "-m",
            "bad-external",
        ],
    );
    let root = f.tmp.path().join("skills");
    fs::create_dir_all(root.join("bad")).unwrap();
    fs::write(root.join("bad/SKILL.md"), "old stays").unwrap();
    f.cmd()
        .args(["install", "--force", "bad"])
        .arg(&root)
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(root.join("bad/SKILL.md")).unwrap(),
        "old stays"
    );
}

#[test]
fn update_all_reports_and_keeps_unrelated() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    f.cmd()
        .args(["install", "--yes", "alpha"])
        .arg(&root)
        .assert()
        .success();
    fs::create_dir_all(root.join("other")).unwrap();
    fs::write(root.join("other/SKILL.md"), "custom").unwrap();
    fs::write(f.catalog.join("skills/alpha/SKILL.md"), "alpha v2").unwrap();
    git(&f.catalog, &["add", "."]);
    git(
        &f.catalog,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "commit",
            "-m",
            "v2",
        ],
    );
    bin()
        .env("SKILLDECK_CONFIG_DIR", f.tmp.path().join("empty-cfg"))
        .arg("update")
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Bulk update complete: 1 updated, 1 skipped",
        ));
    assert_eq!(
        fs::read_to_string(root.join("alpha/SKILL.md")).unwrap(),
        "alpha v2"
    );
    assert_eq!(
        fs::read_to_string(root.join("other/SKILL.md")).unwrap(),
        "custom"
    );
}

#[test]
fn remove_safety_and_group_summary() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    f.cmd()
        .args(["install-group", "--yes", "web"])
        .arg(&root)
        .assert()
        .success();
    fs::create_dir_all(root.join("unsafe")).unwrap();
    f.cmd()
        .args(["remove", "unsafe"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not contain SKILL.md"));
    f.cmd()
        .args(["remove", "alpha"])
        .arg(&root)
        .assert()
        .success();
    f.cmd()
        .args(["remove-group", "web"])
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 removed, 1 skipped"));
}

#[test]
fn direct_git_install_and_bulk_update_uses_provenance() {
    let f = Fixture::new();
    let repo = f.tmp.path().join("direct-skill");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("SKILL.md"), "d1").unwrap();
    commit_repo(&repo);
    let root = f.tmp.path().join("skills");
    let repo_url = file_url(&repo);
    f.cmd()
        .args(["install", "--yes"])
        .arg(&repo_url)
        .arg(&root)
        .assert()
        .success();
    fs::write(repo.join("SKILL.md"), "d2").unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "commit",
            "-m",
            "d2",
        ],
    );
    bin()
        .env("SKILLDECK_CONFIG_DIR", f.tmp.path().join("empty-cfg"))
        .arg("update")
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("direct Git"));
    assert_eq!(
        fs::read_to_string(root.join("direct-skill/SKILL.md")).unwrap(),
        "d2"
    );
}

#[test]
fn install_group_existing_member_yes_overwrites_and_installs_missing() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    f.cmd()
        .args(["install", "--yes", "alpha"])
        .arg(&root)
        .assert()
        .success();
    fs::write(f.catalog.join("skills/alpha/SKILL.md"), "alpha changed").unwrap();
    git(&f.catalog, &["add", "."]);
    git(
        &f.catalog,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "commit",
            "-m",
            "alpha2",
        ],
    );
    f.cmd()
        .args(["install-group", "web"])
        .arg(&root)
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "1 installed, 1 overwritten, 0 skipped",
        ));
    assert_eq!(
        fs::read_to_string(root.join("alpha/SKILL.md")).unwrap(),
        "alpha changed"
    );
    assert!(root.join("ext/SKILL.md").exists());
}

#[test]
fn install_group_existing_member_no_or_eof_skips_and_installs_missing() {
    for stdin in [Some("n\n"), None] {
        let f = Fixture::new();
        let root = f.tmp.path().join("skills");
        f.cmd()
            .args(["install", "--yes", "alpha"])
            .arg(&root)
            .assert()
            .success();
        let mut cmd = f.cmd();
        cmd.args(["install-group", "web"]).arg(&root);
        if let Some(input) = stdin {
            cmd.write_stdin(input);
        }
        cmd.assert().success().stdout(predicate::str::contains(
            "1 installed, 0 overwritten, 1 skipped",
        ));
        assert_eq!(
            fs::read_to_string(root.join("alpha/SKILL.md")).unwrap(),
            "alpha v1"
        );
        assert!(root.join("ext/SKILL.md").exists());
    }
}

#[test]
fn install_group_force_overwrites_all_and_unsafe_existing_is_skipped_without_force() {
    let f = Fixture::new();
    let root = f.tmp.path().join("skills");
    fs::create_dir_all(root.join("alpha")).unwrap();
    fs::write(root.join("alpha/not-a-skill.txt"), "do not delete").unwrap();
    f.cmd()
        .args(["install-group", "web"])
        .arg(&root)
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "1 installed, 0 overwritten, 1 skipped",
        ));
    assert!(root.join("alpha/not-a-skill.txt").exists());
    f.cmd()
        .args(["install-group", "--force", "web"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not contain SKILL.md"));
}

fn git_commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "commit",
            "-m",
            msg,
        ],
    );
}

fn spawn_http(status: u16, body: &'static str) -> String {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            let reason = if status == 200 { "OK" } else { "ERR" };
            let response = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/SKILL.md")
}

#[test]
fn direct_markdown_catalog_http_success_and_error() {
    let tmp = TempDir::new().unwrap();
    let ok_url = spawn_http(200, "markdown skill");
    let catalog = tmp.path().join("catalog");
    fs::create_dir_all(&catalog).unwrap();
    write_external_skills(
        &catalog,
        vec![(
            "md",
            TestExternalSkill {
                source: ok_url,
                subdirectory: None,
                reference: None,
            },
        )],
    );
    no_skill_groups(&catalog);
    commit_repo(&catalog);
    let root = tmp.path().join("skills");
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--yes", "md"])
        .arg(&root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(root.join("md/SKILL.md")).unwrap(),
        "markdown skill"
    );

    let err_url = spawn_http(500, "nope");
    write_external_skills(
        &catalog,
        vec![(
            "md",
            TestExternalSkill {
                source: err_url,
                subdirectory: None,
                reference: None,
            },
        )],
    );
    git_commit_all(&catalog, "error-url");
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--force", "md"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("HTTP status server error"));
    assert_eq!(
        fs::read_to_string(root.join("md/SKILL.md")).unwrap(),
        "markdown skill"
    );
}

#[test]
fn malformed_catalogs_corrupt_manifest_and_unsafe_paths_are_safe() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("skills");
    fs::create_dir_all(root.join("keep")).unwrap();
    fs::write(root.join("keep/SKILL.md"), "keep").unwrap();
    let bad_ext = tmp.path().join("bad-ext");
    fs::create_dir_all(&bad_ext).unwrap();
    fs::write(
        bad_ext.join("external-skills.toml"),
        "[skills.\"x\"\nsource = ",
    )
    .unwrap();
    commit_repo(&bad_ext);
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &bad_ext)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "x"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("parsing"));
    assert_eq!(
        fs::read_to_string(root.join("keep/SKILL.md")).unwrap(),
        "keep"
    );

    let bad_group = tmp.path().join("bad-group");
    fs::create_dir_all(&bad_group).unwrap();
    no_external_skills(&bad_group);
    fs::write(
        bad_group.join("skill-groups.toml"),
        "[groups.\"x\"\nskills = ",
    )
    .unwrap();
    commit_repo(&bad_group);
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &bad_group)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install-group", "x"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("parsing"));

    fs::create_dir_all(root.join(".skilldeck")).unwrap();
    fs::write(root.join(".skilldeck/installations.toml"), "not = [toml").unwrap();
    bin().arg("update").arg(&root).assert().failure();

    let unsafe_subdirs = vec![
        "../outside".to_string(),
        tmp.path().join("absolute-outside").display().to_string(),
    ];
    for subdir in unsafe_subdirs {
        let cat = tmp.path().join(format!("unsafe-{}", subdir.len()));
        fs::create_dir_all(&cat).unwrap();
        write_external_skills(
            &cat,
            vec![(
                "u",
                TestExternalSkill {
                    source: cat.display().to_string(),
                    subdirectory: Some(subdir.clone()),
                    reference: None,
                },
            )],
        );
        no_skill_groups(&cat);
        commit_repo(&cat);
        bin()
            .env("SKILLDECK_CATALOG_REPOSITORY", &cat)
            .env("SKILLDECK_CATALOG_REF", "master")
            .args(["install", "u"])
            .arg(&root)
            .assert()
            .failure()
            .stderr(predicate::str::contains("unsafe"));
    }
}

#[test]
fn git_ref_tracking_and_pinning_semantics() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("source");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("SKILL.md"), "v1").unwrap();
    commit_repo(&repo);
    let pinned = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    git(&repo, &["checkout", "-b", "feature"]);
    fs::write(repo.join("SKILL.md"), "branch v1").unwrap();
    git_commit_all(&repo, "branch1");
    git(&repo, &["checkout", "master"]);

    let catalog = tmp.path().join("catalog");
    fs::create_dir_all(&catalog).unwrap();
    write_external_skills(
        &catalog,
        vec![
            (
                "default",
                TestExternalSkill {
                    source: repo.display().to_string(),
                    subdirectory: None,
                    reference: None,
                },
            ),
            (
                "dash",
                TestExternalSkill {
                    source: repo.display().to_string(),
                    subdirectory: None,
                    reference: Some("-".into()),
                },
            ),
            (
                "pinned",
                TestExternalSkill {
                    source: repo.display().to_string(),
                    subdirectory: None,
                    reference: Some(pinned),
                },
            ),
            (
                "branch",
                TestExternalSkill {
                    source: repo.display().to_string(),
                    subdirectory: None,
                    reference: Some("feature".into()),
                },
            ),
        ],
    );
    no_skill_groups(&catalog);
    commit_repo(&catalog);
    let root = tmp.path().join("skills");
    for skill in ["default", "dash", "pinned", "branch"] {
        bin()
            .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
            .env("SKILLDECK_CATALOG_REF", "master")
            .args(["install", "--yes", skill])
            .arg(&root)
            .assert()
            .success();
    }
    fs::write(repo.join("SKILL.md"), "v2").unwrap();
    git_commit_all(&repo, "v2");
    git(&repo, &["checkout", "feature"]);
    fs::write(repo.join("SKILL.md"), "branch v2").unwrap();
    git_commit_all(&repo, "branch2");
    git(&repo, &["checkout", "master"]);
    bin().arg("update").arg(&root).assert().success();
    assert_eq!(
        fs::read_to_string(root.join("default/SKILL.md")).unwrap(),
        "v2"
    );
    assert_eq!(
        fs::read_to_string(root.join("dash/SKILL.md")).unwrap(),
        "v2"
    );
    assert_eq!(
        fs::read_to_string(root.join("pinned/SKILL.md")).unwrap(),
        "v1"
    );
    assert_eq!(
        fs::read_to_string(root.join("branch/SKILL.md")).unwrap(),
        "branch v2"
    );
}

#[test]
fn single_updates_missing_refs_failed_clones_and_force_unsafe_destinations() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("direct-one");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("SKILL.md"), "d1").unwrap();
    commit_repo(&repo);
    let root = tmp.path().join("skills");
    let url = file_url(&repo);
    bin()
        .args(["install", "--yes"])
        .arg(&url)
        .arg(&root)
        .assert()
        .success();
    fs::write(repo.join("SKILL.md"), "d2").unwrap();
    git_commit_all(&repo, "d2");
    bin().arg("update").arg(&url).arg(&root).assert().success();
    assert_eq!(
        fs::read_to_string(root.join("direct-one/SKILL.md")).unwrap(),
        "d2"
    );

    let cat = make_catalog(tmp.path(), "cat-single", "alpha", "a1");
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &cat)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--yes", "alpha"])
        .arg(&root)
        .assert()
        .success();
    fs::write(cat.join("skills/alpha/SKILL.md"), "a2").unwrap();
    git_commit_all(&cat, "a2");
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &cat)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["update", "alpha"])
        .arg(&root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(root.join("alpha/SKILL.md")).unwrap(),
        "a2"
    );

    bin()
        .args(["install", "--yes"])
        .arg(file_url(&tmp.path().join("definitely-missing-repo")))
        .arg(tmp.path().join("badroot"))
        .assert()
        .failure();
    let badref = tmp.path().join("badref");
    fs::create_dir_all(&badref).unwrap();
    write_external_skills(
        &badref,
        vec![(
            "x",
            TestExternalSkill {
                source: repo.display().to_string(),
                subdirectory: None,
                reference: Some("missing-ref".into()),
            },
        )],
    );
    no_skill_groups(&badref);
    commit_repo(&badref);
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &badref)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "x"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing-ref"));

    fs::create_dir_all(root.join("unsafe-single")).unwrap();
    fs::write(root.join("unsafe-single/file"), "keep").unwrap();
    let unsafe_cat = make_catalog(tmp.path(), "unsafe-single-cat", "unsafe-single", "new");
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &unsafe_cat)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--force", "unsafe-single"])
        .arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to replace"));
    assert_eq!(
        fs::read_to_string(root.join("unsafe-single/file")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn symlink_destination_and_paths_with_spaces_do_not_modify_outside_target() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let catalog = make_catalog(tmp.path(), "catalog with spaces", "space-skill", "inside");
    let root = tmp.path().join("install root with spaces");
    fs::create_dir_all(&root).unwrap();
    let outside = tmp.path().join("outside target");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), "outside").unwrap();
    symlink(&outside, root.join("space-skill")).unwrap();
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install", "--force", "space-skill"])
        .arg(&root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(outside.join("SKILL.md")).unwrap(),
        "outside"
    );
    assert_eq!(
        fs::read_to_string(root.join("space-skill/SKILL.md")).unwrap(),
        "inside"
    );
}

#[test]
fn install_group_source_failure_happens_before_destructive_overwrite() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "source").unwrap();
    commit_repo(&source);
    let catalog = tmp.path().join("catalog");
    fs::create_dir_all(catalog.join("skills/alpha")).unwrap();
    fs::write(catalog.join("skills/alpha/SKILL.md"), "new alpha").unwrap();
    write_external_skills(
        &catalog,
        vec![(
            "bad",
            TestExternalSkill {
                source: source.display().to_string(),
                subdirectory: None,
                reference: Some("missing-ref".into()),
            },
        )],
    );
    write_skill_groups(&catalog, vec![("g", "alpha bad")]);
    commit_repo(&catalog);
    let root = tmp.path().join("skills");
    fs::create_dir_all(root.join("alpha")).unwrap();
    fs::write(root.join("alpha/SKILL.md"), "old alpha").unwrap();
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["install-group", "g"])
        .arg(&root)
        .write_stdin("y\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing-ref"));
    assert_eq!(
        fs::read_to_string(root.join("alpha/SKILL.md")).unwrap(),
        "old alpha"
    );
    assert!(!root.join("bad").exists());
}

#[test]
fn init_validates_catalog_prints_summary_and_preserves_old_config_on_failure() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let good = Fixture::new();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--repository"])
        .arg(&good.catalog)
        .args(["--reference", "master"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Found 3 skills and 1 groups"));
    let config_path = cfg.join("config.toml");
    let before = fs::read(&config_path).unwrap();

    let bad = tmp.path().join("bad-missing-skill-md");
    fs::create_dir_all(bad.join("skills/broken")).unwrap();
    no_external_skills(&bad);
    no_skill_groups(&bad);
    commit_repo(&bad);
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--force", "--repository"])
        .arg(&bad)
        .args(["--reference", "master"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("zero skills"));
    assert_eq!(fs::read(&config_path).unwrap(), before);

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--force", "--repository"])
        .arg(&good.catalog)
        .args(["--reference", "missing-ref"])
        .assert()
        .failure();
    assert_eq!(fs::read(&config_path).unwrap(), before);
}

type CatalogBuilder = Box<dyn Fn(&Path)>;

#[test]
fn init_rejects_structural_catalog_errors_and_preserves_config() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let good = make_catalog(tmp.path(), "good", "ok", "ok");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--repository"])
        .arg(&good)
        .args(["--reference", "master"])
        .assert()
        .success();
    let config_path = cfg.join("config.toml");
    let before = fs::read(&config_path).unwrap();

    let cases: Vec<(&str, CatalogBuilder)> = vec![
        (
            "malformed-ext",
            Box::new(|c| {
                fs::create_dir_all(c.join("skills/ok")).unwrap();
                fs::write(c.join("skills/ok/SKILL.md"), "ok").unwrap();
                fs::write(c.join("external-skills.toml"), "[skills.\"x\"\nsource = ").unwrap();
                no_skill_groups(c);
            }),
        ),
        (
            "malformed-groups",
            Box::new(|c| {
                fs::create_dir_all(c.join("skills/ok")).unwrap();
                fs::write(c.join("skills/ok/SKILL.md"), "ok").unwrap();
                no_external_skills(c);
                fs::write(c.join("skill-groups.toml"), "[groups.\"x\"\nskills = ").unwrap();
            }),
        ),
        (
            "unsafe-path",
            Box::new(|c| {
                fs::create_dir_all(c).unwrap();
                write_external_skills(
                    c,
                    vec![(
                        "x",
                        TestExternalSkill {
                            source: "repo".into(),
                            subdirectory: Some("../x".into()),
                            reference: None,
                        },
                    )],
                );
                no_skill_groups(c);
            }),
        ),
        (
            "empty-source",
            Box::new(|c| {
                fs::create_dir_all(c).unwrap();
                write_external_skills(
                    c,
                    vec![(
                        "x",
                        TestExternalSkill {
                            source: "".into(),
                            subdirectory: None,
                            reference: None,
                        },
                    )],
                );
                no_skill_groups(c);
            }),
        ),
        (
            "duplicate",
            Box::new(|c| {
                fs::create_dir_all(c.join("skills/x")).unwrap();
                fs::write(c.join("skills/x/SKILL.md"), "x").unwrap();
                write_external_skills(
                    c,
                    vec![(
                        "x",
                        TestExternalSkill {
                            source: "repo".into(),
                            subdirectory: None,
                            reference: None,
                        },
                    )],
                );
                no_skill_groups(c);
            }),
        ),
        (
            "empty-group",
            Box::new(|c| {
                fs::create_dir_all(c.join("skills/x")).unwrap();
                fs::write(c.join("skills/x/SKILL.md"), "x").unwrap();
                no_external_skills(c);
                write_skill_groups(c, vec![("g", "")]);
            }),
        ),
        (
            "dup-member",
            Box::new(|c| {
                fs::create_dir_all(c.join("skills/x")).unwrap();
                fs::write(c.join("skills/x/SKILL.md"), "x").unwrap();
                no_external_skills(c);
                write_skill_groups(c, vec![("g", "x x")]);
            }),
        ),
        (
            "missing-member",
            Box::new(|c| {
                fs::create_dir_all(c.join("skills/x")).unwrap();
                fs::write(c.join("skills/x/SKILL.md"), "x").unwrap();
                no_external_skills(c);
                write_skill_groups(c, vec![("g", "missing")]);
            }),
        ),
        (
            "zero",
            Box::new(|c| {
                fs::create_dir_all(c).unwrap();
                no_external_skills(c);
                no_skill_groups(c);
            }),
        ),
    ];

    for (name, build) in cases {
        let cat = tmp.path().join(name);
        build(&cat);
        commit_repo(&cat);
        bin()
            .env("SKILLDECK_CONFIG_DIR", &cfg)
            .args(["init", "--yes", "--force", "--repository"])
            .arg(&cat)
            .args(["--reference", "master"])
            .assert()
            .failure();
        assert_eq!(
            fs::read(&config_path).unwrap(),
            before,
            "{name} changed config"
        );
    }
}

#[test]
fn list_human_json_and_precedence_are_sorted() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let config_cat = make_catalog(tmp.path(), "config-cat", "config-only", "c");
    let env_cat = make_catalog(tmp.path(), "env-cat", "env-only", "e");
    let cli_cat = make_catalog(tmp.path(), "cli-cat", "cli-only", "c");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--repository"])
        .arg(&config_cat)
        .args(["--reference", "master"])
        .assert()
        .success();

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .env("SKILLDECK_CATALOG_REPOSITORY", &env_cat)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("env-only"))
        .stdout(predicate::str::contains("[first-party]"));

    let out = bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .env("SKILLDECK_CATALOG_REPOSITORY", &env_cat)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["list", "--catalog-repository"])
        .arg(&cli_cat)
        .args(["--catalog-ref", "master", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["skills"][0]["name"], "cli-only");
    assert_eq!(json["counts"]["total_skill_count"], 1);
}

#[test]
fn doctor_structural_and_deep_validation() {
    let tmp = TempDir::new().unwrap();
    let ext_repo = tmp.path().join("extrepo");
    fs::create_dir_all(&ext_repo).unwrap();
    fs::write(ext_repo.join("SKILL.md"), "ext").unwrap();
    commit_repo(&ext_repo);
    let md_url = spawn_http(200, "# skill");
    let catalog = tmp.path().join("doctor-cat");
    fs::create_dir_all(catalog.join("skills/local")).unwrap();
    fs::write(catalog.join("skills/local/SKILL.md"), "local").unwrap();
    write_external_skills(
        &catalog,
        vec![
            (
                "gitext",
                TestExternalSkill {
                    source: ext_repo.display().to_string(),
                    subdirectory: None,
                    reference: None,
                },
            ),
            (
                "md",
                TestExternalSkill {
                    source: md_url,
                    subdirectory: None,
                    reference: None,
                },
            ),
        ],
    );
    write_skill_groups(&catalog, vec![("all", "local gitext md")]);
    commit_repo(&catalog);
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("Catalog structure: ok"));
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["doctor", "--deep"])
        .assert()
        .success()
        .stdout(predicate::str::contains("External gitext: ok"));

    let bad = tmp.path().join("bad-deep");
    fs::create_dir_all(&bad).unwrap();
    write_external_skills(
        &bad,
        vec![(
            "bad",
            TestExternalSkill {
                source: ext_repo.display().to_string(),
                subdirectory: Some("missing".into()),
                reference: None,
            },
        )],
    );
    no_skill_groups(&bad);
    commit_repo(&bad);
    bin()
        .env("SKILLDECK_CATALOG_REPOSITORY", &bad)
        .env("SKILLDECK_CATALOG_REF", "master")
        .args(["doctor", "--deep"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("subdirectory"));
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "skilldeck.exe"
    } else {
        "skilldeck"
    }
}

fn zip_with_skilldeck(bytes: &[u8]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::FileOptions::<()>::default();
        zip.start_file(exe_name(), opts).unwrap();
        zip.write_all(bytes).unwrap();
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn platform_archive_with_skilldeck(bytes: &[u8]) -> (String, Vec<u8>) {
    let asset = if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "skilldeck-x86_64-unknown-linux-gnu.tar.xz"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "skilldeck-aarch64-unknown-linux-gnu.tar.xz"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "skilldeck-x86_64-apple-darwin.tar.xz"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "skilldeck-aarch64-apple-darwin.tar.xz"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "skilldeck-x86_64-pc-windows-msvc.zip"
    } else {
        panic!("unsupported test target")
    };
    if asset.ends_with(".zip") {
        return (asset.to_string(), zip_with_skilldeck(bytes));
    }
    let mut out = Vec::new();
    {
        let enc = xz2::write::XzEncoder::new(&mut out, 6);
        let mut tar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("skilldeck/{}/{}", "bin", exe_name()),
            bytes,
        )
        .unwrap();
        let enc = tar.into_inner().unwrap();
        enc.finish().unwrap();
    }
    (asset.to_string(), out)
}

struct TestServer {
    url: String,
    hits: Arc<Mutex<Vec<String>>>,
    alive: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn new(routes: Vec<(&'static str, Vec<u8>, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes: Vec<_> = routes
            .into_iter()
            .map(|(p, b, ct)| {
                let body = if let Ok(text) = String::from_utf8(b.clone()) {
                    text.replace("http://placeholder", &url).into_bytes()
                } else {
                    b
                };
                (p.to_string(), body, ct.to_string())
            })
            .collect();
        listener.set_nonblocking(true).unwrap();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let thread_hits = hits.clone();
        let thread_alive = alive.clone();
        let handle = thread::spawn(move || {
            while thread_alive.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                thread_hits.lock().unwrap().push(path.clone());
                let route = routes.iter().find(|(p, _, _)| *p == path);
                if let Some((_, body, ct)) = route {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        ct,
                        body.len()
                    ).unwrap();
                    stream.write_all(body).unwrap();
                } else {
                    let body = b"not found";
                    write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                }
            }
        });
        Self {
            url,
            hits,
            alive,
            handle: Some(handle),
        }
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn release_json(base: &str, tag: &str, draft: bool, prerelease: bool, asset: &str) -> String {
    format!(
        r#"[{{"tag_name":"v9.0.0","draft":true,"prerelease":false,"assets":[]}},{{"tag_name":"v8.0.0","draft":false,"prerelease":true,"assets":[]}},{{"tag_name":"{}","draft":{},"prerelease":{},"assets":[{{"name":"{}","browser_download_url":"{}/archive"}},{{"name":"{}.sha256","browser_download_url":"{}/archive.sha256"}}]}}]"#,
        tag, draft, prerelease, asset, base, asset, base
    )
}

#[test]
fn upgrade_check_current_and_available_exit_success_without_download() {
    let tmp = TempDir::new().unwrap();
    let asset = "skilldeck-test.zip";
    let server = TestServer::new(vec![(
        "/releases",
        release_json("http://placeholder", "v0.1.2", false, false, asset).into_bytes(),
        "application/json",
    )]);
    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache"))
        .args(["upgrade", "--check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
    assert!(!server.hits().contains(&"/archive".to_string()));

    let server = TestServer::new(vec![(
        "/releases",
        release_json("http://placeholder", "v0.1.3", false, false, asset).into_bytes(),
        "application/json",
    )]);
    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache2"))
        .args(["upgrade", "--check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Update available"))
        .stdout(predicate::str::contains("Run `skilldeck upgrade`"));
    assert!(!server.hits().contains(&"/archive".to_string()));
}

#[test]
fn upgrade_actual_current_exe_self_replace_path_keeps_copied_binary_runnable() {
    let tmp = TempDir::new().unwrap();
    let source = assert_cmd::cargo::cargo_bin("skilldeck");
    let copied = tmp.path().join(exe_name());
    fs::copy(&source, &copied).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&source).unwrap().permissions().mode();
        fs::set_permissions(&copied, fs::Permissions::from_mode(mode)).unwrap();
    }
    let bytes = fs::read(&source).unwrap();
    let (asset, archive) = platform_archive_with_skilldeck(&bytes);
    let checksum = format!("{}  {asset}\n", sha256_hex(&archive));
    let server = TestServer::new(vec![
        (
            "/releases",
            release_json("http://placeholder", "v0.1.3", false, false, &asset).into_bytes(),
            "application/json",
        ),
        ("/archive", archive, "application/octet-stream"),
        (
            "/archive.sha256",
            checksum.as_bytes().to_vec(),
            "text/plain",
        ),
    ]);
    Command::new(&copied)
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache"))
        .args(["upgrade", "--yes"])
        .output()
        .map(|output| {
            assert!(
                output.status.success(),
                "upgrade failed\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        })
        .unwrap();
    let output = Command::new(&copied).arg("version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn upgrade_no_eof_preserves_target_and_yes_replaces_after_checksum() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(if cfg!(windows) {
        "skilldeck.exe"
    } else {
        "skilldeck"
    });
    fs::write(&target, b"old-binary").unwrap();
    let archive = zip_with_skilldeck(b"new-binary");
    let checksum = format!("{}  skilldeck-test.zip\n", sha256_hex(&archive));
    let asset = "skilldeck-test.zip";

    let server = TestServer::new(vec![
        (
            "/releases",
            release_json("http://placeholder", "v0.1.3", false, false, asset).into_bytes(),
            "application/json",
        ),
        ("/archive", archive.clone(), "application/octet-stream"),
        (
            "/archive.sha256",
            checksum.as_bytes().to_vec(),
            "text/plain",
        ),
    ]);
    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_UPGRADE_EXE", &target)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache"))
        .arg("upgrade")
        .assert()
        .success()
        .stdout(predicate::str::contains("Upgrade cancelled"));
    assert_eq!(fs::read(&target).unwrap(), b"old-binary");

    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_UPGRADE_EXE", &target)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache"))
        .args(["upgrade", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Upgraded Skilldeck"));
    assert_eq!(fs::read(&target).unwrap(), b"new-binary");
}

#[test]
fn upgrade_readonly_target_fails_with_package_manager_caveat() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(if cfg!(windows) {
        "skilldeck.exe"
    } else {
        "skilldeck"
    });
    fs::write(&target, b"old").unwrap();
    let original_perms = fs::metadata(&target).unwrap().permissions();
    let mut perms = original_perms.clone();
    perms.set_readonly(true);
    fs::set_permissions(&target, perms).unwrap();
    let archive = zip_with_skilldeck(b"new");
    let checksum = format!("{}  skilldeck-test.zip\n", sha256_hex(&archive));
    let asset = "skilldeck-test.zip";
    let server = TestServer::new(vec![
        (
            "/releases",
            release_json("http://placeholder", "v0.1.3", false, false, asset).into_bytes(),
            "application/json",
        ),
        ("/archive", archive, "application/octet-stream"),
        (
            "/archive.sha256",
            checksum.as_bytes().to_vec(),
            "text/plain",
        ),
    ]);
    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_UPGRADE_EXE", &target)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache"))
        .args(["upgrade", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("package manager"));
    fs::set_permissions(&target, original_perms).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"old");
}

#[test]
fn upgrade_checksum_mismatch_and_http_failures_preserve_target() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(if cfg!(windows) {
        "skilldeck.exe"
    } else {
        "skilldeck"
    });
    fs::write(&target, b"old").unwrap();
    let archive = zip_with_skilldeck(b"new");
    let asset = "skilldeck-test.zip";
    let server = TestServer::new(vec![
        (
            "/releases",
            release_json("http://placeholder", "v0.1.3", false, false, asset).into_bytes(),
            "application/json",
        ),
        ("/archive", archive, "application/octet-stream"),
        (
            "/archive.sha256",
            b"deadbeef  skilldeck-test.zip\n".to_vec(),
            "text/plain",
        ),
    ]);
    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_UPGRADE_EXE", &target)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache"))
        .args(["upgrade", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("checksum mismatch"));
    assert_eq!(fs::read(&target).unwrap(), b"old");

    let server = TestServer::new(vec![("/releases", b"[]".to_vec(), "application/json")]);
    bin()
        .env("SKILLDECK_UPGRADE_BASE_URL", &server.url)
        .env("SKILLDECK_UPGRADE_ASSET", asset)
        .env("SKILLDECK_UPGRADE_EXE", &target)
        .env("SKILLDECK_CACHE_DIR", tmp.path().join("cache2"))
        .args(["upgrade", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no stable"));
    assert_eq!(fs::read(&target).unwrap(), b"old");
}
