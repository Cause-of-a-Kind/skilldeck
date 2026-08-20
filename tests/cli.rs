use std::{collections::BTreeMap, fs, path::Path, process::Command};

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
