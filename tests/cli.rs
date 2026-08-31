use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
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

fn bootstrap_bin() -> assert_cmd::Command {
    let mut cmd = bin();
    cmd.env("GIT_AUTHOR_NAME", "Skilldeck Test")
        .env("GIT_AUTHOR_EMAIL", "skilldeck-test@example.com")
        .env("GIT_COMMITTER_NAME", "Skilldeck Test")
        .env("GIT_COMMITTER_EMAIL", "skilldeck-test@example.com");
    cmd
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

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn assert_bootstrap_git_repo(dest: &Path) {
    assert!(dest.join(".git").is_dir());
    assert_eq!(git_stdout(dest, &["branch", "--show-current"]), "main");
    assert_eq!(git_stdout(dest, &["rev-list", "--count", "HEAD"]), "1");
    assert_eq!(
        git_stdout(dest, &["log", "-1", "--pretty=%s"]),
        "Start Skilldeck catalog"
    );
    assert!(git_stdout(dest, &["status", "--porcelain"]).is_empty());
    assert!(git_stdout(dest, &["ls-files"]).contains("README.md"));
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
fn embedded_docs_match_the_active_binary_and_cover_agent_recipes() {
    bin().arg("docs").assert().success().stdout(
        predicate::str::contains(env!("CARGO_PKG_VERSION"))
            .and(predicate::str::contains("agent"))
            .and(predicate::str::contains("recipes")),
    );
    bin().args(["docs", "agent"]).assert().success().stdout(
        predicate::str::contains("name: skilldeck")
            .and(predicate::str::contains("skilldeck docs recipes"))
            .and(predicate::str::contains("--target pi|codex|claude")),
    );
    bin()
        .args(["docs", "recipes"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("### Composable skill recipes")
                .and(predicate::str::contains("MiniJinja"))
                .and(predicate::str::contains("local_inputs"))
                .and(predicate::str::contains("upstream.frontmatter")),
        )
        .stdout(predicate::str::contains("### Maintain a local catalog").not());
    bin()
        .args(["docs", "readme"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("# Skilldeck"));
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
                let (stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                handle_test_server_client(stream, &routes, &thread_hits);
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

fn handle_test_server_client(
    mut stream: TcpStream,
    routes: &[(String, Vec<u8>, String)],
    hits: &Arc<Mutex<Vec<String>>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 16 * 1024 {
                    break;
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if request.is_empty() {
                    return;
                }
                break;
            }
            Err(_) => return,
        }
    }

    let req = String::from_utf8_lossy(&request);
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    hits.lock().unwrap().push(path.clone());

    let (status, body, ct): (&str, &[u8], &str) =
        if let Some((_, body, ct)) = routes.iter().find(|(p, _, _)| *p == path) {
            ("200 OK", body.as_slice(), ct.as_str())
        } else {
            ("404 Not Found", b"not found".as_slice(), "text/plain")
        };
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {ct}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = write_http_bytes(&mut stream, head.as_bytes());
    let _ = write_http_bytes(&mut stream, body);
    let _ = stream.flush();
}

fn write_http_bytes(stream: &mut TcpStream, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        match stream.write(bytes) {
            Ok(0) => return Ok(()),
            Ok(n) => bytes = &bytes[n..],
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(Duration::from_millis(1));
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionReset
                ) =>
            {
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn test_server_serves_complete_response_and_survives_disconnects() {
    let body = b"hello from test server".to_vec();
    let server = TestServer::new(vec![("/ok", body.clone(), "text/plain")]);

    let _ = TcpStream::connect(server.url.trim_start_matches("http://"));

    let mut stream = TcpStream::connect(server.url.trim_start_matches("http://")).unwrap();
    stream
        .write_all(b"GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(
        text.contains(&format!("content-length: {}", body.len())),
        "{text}"
    );
    assert!(response.ends_with(&body), "{text}");
    assert!(server.hits().contains(&"/ok".to_string()));
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
        release_json(
            "http://placeholder",
            &format!("v{}", env!("CARGO_PKG_VERSION")),
            false,
            false,
            asset,
        )
        .into_bytes(),
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
        release_json("http://placeholder", "v99.0.0", false, false, asset).into_bytes(),
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
            release_json("http://placeholder", "v99.0.0", false, false, &asset).into_bytes(),
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
            release_json("http://placeholder", "v99.0.0", false, false, asset).into_bytes(),
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
            release_json("http://placeholder", "v99.0.0", false, false, asset).into_bytes(),
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
            release_json("http://placeholder", "v99.0.0", false, false, asset).into_bytes(),
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

#[test]
fn bootstrap_quickstart_explicit_creates_expected_catalog_without_config() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("my catalog");
    let cfg = tmp.path().join("cfg");
    bootstrap_bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["bootstrap", dest.to_str().unwrap(), "--quickstart"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created Skilldeck catalog"))
        .stdout(predicate::str::contains(
            "Git repository initialized on branch main",
        ))
        .stdout(predicate::str::contains(
            "initial commit `Start Skilldeck catalog`",
        ))
        .stdout(predicate::str::contains("Next steps:"))
        .stdout(predicate::str::contains("optionally add a remote"))
        .stdout(predicate::str::contains(
            "From inside the generated directory",
        ))
        .stdout(predicate::str::contains(
            "skilldeck init --repository . --reference main",
        ))
        .stdout(predicate::str::contains("skilldeck doctor"))
        .stdout(predicate::str::contains(
            "skilldeck install-group quickstart <install-directory>",
        ))
        .stdout(predicate::str::contains("git init --initial-branch=main").not())
        .stdout(predicate::str::contains("cd ").not());

    assert!(!cfg.exists(), "bootstrap must not mutate global config");
    let readme = fs::read_to_string(dest.join("README.md")).unwrap();
    assert!(readme.contains("git init --initial-branch=main"));
    assert!(readme.contains("--no-git"));
    assert!(dest.join("README.md").is_file());
    let skill = fs::read_to_string(dest.join("skills/hello-world/SKILL.md")).unwrap();
    assert!(skill.contains("name: hello-world"));
    assert!(skill.contains("description:"));
    let external = fs::read_to_string(dest.join("external-skills.toml")).unwrap();
    assert!(external.contains("https://github.com/Cause-of-a-Kind/skilldeck.git"));
    assert!(external.contains("subdirectory = \"examples/skilldeck-skill\""));
    assert!(external.contains("ref = \"v0.2.0\""));
    assert_eq!(
        fs::read_to_string(dest.join("skill-groups.toml")).unwrap(),
        "[groups.quickstart]\nskills = \"hello-world skilldeck\"\n"
    );
    assert_bootstrap_git_repo(&dest);

    bin()
        .env("SKILLDECK_CONFIG_DIR", tmp.path().join("cfg2"))
        .env("SKILLDECK_NO_UPDATE_CHECK", "1")
        .args([
            "doctor",
            "--catalog-repository",
            dest.to_str().unwrap(),
            "--catalog-ref",
            "main",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Found 2 skills and 1 groups"));
}

#[test]
fn bootstrap_empty_explicit_accepts_existing_empty_directory() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("empty-dest");
    fs::create_dir(&dest).unwrap();
    bootstrap_bin()
        .args(["bootstrap", dest.to_str().unwrap(), "--empty"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Git repository initialized on branch main",
        ))
        .stdout(predicate::str::contains("install-group quickstart").not());
    assert!(dest.join("skills/.gitkeep").is_file());
    assert!(fs::read_to_string(dest.join("external-skills.toml"))
        .unwrap()
        .contains("[skills.example]"));
    assert!(fs::read_to_string(dest.join("skill-groups.toml"))
        .unwrap()
        .contains("[groups.default]"));
    toml::from_str::<toml::Value>(&fs::read_to_string(dest.join("external-skills.toml")).unwrap())
        .unwrap();
    toml::from_str::<toml::Value>(&fs::read_to_string(dest.join("skill-groups.toml")).unwrap())
        .unwrap();
    assert_bootstrap_git_repo(&dest);
}

#[test]
fn bootstrap_no_git_generates_files_without_repository_and_manual_output() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("no-git-catalog");
    bin()
        .args([
            "bootstrap",
            dest.to_str().unwrap(),
            "--quickstart",
            "--no-git",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Git initialization skipped"))
        .stdout(predicate::str::contains("git init --initial-branch=main"))
        .stdout(predicate::str::contains(
            "git commit -m \"Start Skilldeck catalog\"",
        ));
    assert!(dest.join("skills/hello-world/SKILL.md").is_file());
    assert!(!dest.join(".git").exists());
}

#[test]
fn bootstrap_git_identity_failure_preserves_initialized_staged_repo() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("identity-failure");
    let home = tmp.path().join("home");
    fs::create_dir(&home).unwrap();
    bin()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("GIT_CONFIG_GLOBAL", tmp.path().join("missing-gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.useConfigOnly")
        .env("GIT_CONFIG_VALUE_0", "true")
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env_remove("EMAIL")
        .args(["bootstrap", dest.to_str().unwrap(), "--empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git commit failed"))
        .stderr(predicate::str::contains(
            "Catalog files exist and Git is initialized/staged",
        ))
        .stderr(predicate::str::contains(
            "because Git identity is not configured",
        ))
        .stderr(predicate::str::contains(
            "From inside the generated repository",
        ))
        .stderr(predicate::str::contains("git config user.name"))
        .stderr(predicate::str::contains("git config user.email"))
        .stderr(predicate::str::contains(
            "git commit -m \"Start Skilldeck catalog\"",
        ));

    assert!(dest.join(".git").is_dir());
    assert!(dest.join("README.md").is_file());
    let status = git_stdout(&dest, &["status", "--porcelain"]);
    assert!(status.lines().any(|line| line.starts_with('A')), "{status}");
    let head = Command::new("git")
        .arg("-C")
        .arg(&dest)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .unwrap();
    assert!(!head.status.success(), "commit should not exist");
}

#[test]
fn bootstrap_generic_commit_failure_preserves_staged_repo_without_identity_advice() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("signing-failure");
    bin()
        .env("GIT_AUTHOR_NAME", "Skilldeck Test")
        .env("GIT_AUTHOR_EMAIL", "skilldeck-test@example.com")
        .env("GIT_COMMITTER_NAME", "Skilldeck Test")
        .env("GIT_COMMITTER_EMAIL", "skilldeck-test@example.com")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "true")
        .env("GIT_CONFIG_KEY_1", "gpg.program")
        .env("GIT_CONFIG_VALUE_1", "false")
        .args(["bootstrap", dest.to_str().unwrap(), "--empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git commit failed"))
        .stderr(predicate::str::contains(
            "Catalog files exist and Git is initialized/staged",
        ))
        .stderr(predicate::str::contains("Fix the Git error"))
        .stderr(predicate::str::contains(
            "git commit -m \"Start Skilldeck catalog\"",
        ))
        .stderr(predicate::str::contains("git config user.name").not())
        .stderr(predicate::str::contains("git config user.email").not());

    assert!(dest.join(".git").is_dir());
    let status = git_stdout(&dest, &["status", "--porcelain"]);
    assert!(status.lines().any(|line| line.starts_with('A')), "{status}");
}

#[test]
fn bootstrap_refuses_non_empty_without_mutation() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("catalog");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("keep.txt"), "keep").unwrap();
    bin()
        .args(["bootstrap", dest.to_str().unwrap(), "--quickstart"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
    assert_eq!(fs::read_to_string(dest.join("keep.txt")).unwrap(), "keep");
    assert!(!dest.join("README.md").exists());
}

#[test]
fn bootstrap_noninteractive_missing_inputs_and_conflicts_fail() {
    bin()
        .arg("bootstrap")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "non-interactive bootstrap requires",
        ));
    let tmp = TempDir::new().unwrap();
    bin()
        .args(["bootstrap", tmp.path().join("x").to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "non-interactive bootstrap requires",
        ));
    bin()
        .args([
            "bootstrap",
            tmp.path().join("x").to_str().unwrap(),
            "--quickstart",
            "--empty",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn bootstrap_interactive_defaults_and_empty_choice_work_with_stdin() {
    let tmp = TempDir::new().unwrap();
    let mut cmd = bootstrap_bin();
    cmd.current_dir(tmp.path())
        .arg("bootstrap")
        .write_stdin("\nnope\n2\n")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Where should the catalog be created? [./skilldeck-catalog]",
        ))
        .stderr(predicate::str::contains("Quickstart"))
        .stderr(predicate::str::contains(
            "Unknown bootstrap template `nope`",
        ));
    let default_dest = tmp.path().join("skilldeck-catalog");
    assert!(default_dest.join("skills/.gitkeep").is_file());
    assert_bootstrap_git_repo(&default_dest);

    let dest = tmp.path().join("chosen empty");
    bootstrap_bin()
        .arg("bootstrap")
        .write_stdin(format!("{}\nempty\n", dest.display()))
        .assert()
        .success();
    assert!(dest.join("skills/.gitkeep").is_file());
    assert_bootstrap_git_repo(&dest);
}

#[cfg(unix)]
#[test]
fn bootstrap_refuses_symlink_destination() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real");
    let link = tmp.path().join("link");
    fs::create_dir(&real).unwrap();
    symlink(&real, &link).unwrap();
    bin()
        .args(["bootstrap", link.to_str().unwrap(), "--empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("symlink"));
    assert!(fs::read_dir(&real).unwrap().next().is_none());
}

#[test]
fn public_skilldeck_example_skill_exists_and_mentions_safe_commands() {
    let body = fs::read_to_string("examples/skilldeck-skill/SKILL.md").unwrap();
    assert!(body.contains("description:"));
    for needle in [
        "skilldeck list",
        "skilldeck install",
        "skilldeck install-group",
        "skilldeck update",
        "skilldeck doctor",
        "skilldeck remove",
        "skilldeck upgrade",
        "Update vs upgrade",
        "--force",
        "git init --initial-branch=main",
        "--no-git",
    ] {
        assert!(body.contains(needle), "missing {needle}");
    }
}

#[test]
fn bootstrap_creates_nested_paths_with_spaces() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("nested parent/catalog with spaces");
    bootstrap_bin()
        .args(["bootstrap", dest.to_str().unwrap(), "--quickstart"])
        .assert()
        .success()
        .stdout(predicate::str::contains(dest.display().to_string()))
        .stdout(predicate::str::contains("cd ").not());
    assert!(dest.join("skills/hello-world/SKILL.md").is_file());
    assert_bootstrap_git_repo(&dest);
    let parent = tmp.path().join("nested parent");
    assert!(fs::read_dir(parent).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".skilldeck-bootstrap-")));
}

#[test]
fn multiple_registries_support_qualified_names_defaults_and_listing() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let company = make_catalog(tmp.path(), "company", "shared", "from company");
    let personal = make_catalog(tmp.path(), "personal", "shared", "from personal");

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--name", "company", "--repository"])
        .arg(&company)
        .args(["--reference", "master"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "add", "personal"])
        .arg(&personal)
        .args(["--reference", "master", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Default registry: company"));

    let default_root = tmp.path().join("default-root");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install", "--yes", "shared"])
        .arg(&default_root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(default_root.join("shared/SKILL.md")).unwrap(),
        "from company"
    );

    let personal_root = tmp.path().join("personal-root");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install", "--yes", "personal:shared"])
        .arg(&personal_root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(personal_root.join("shared/SKILL.md")).unwrap(),
        "from personal"
    );

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("company:shared"))
        .stdout(predicate::str::contains("personal:shared"));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "default", "personal"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "rename", "personal", "mine"])
        .assert()
        .success();
    let config = fs::read_to_string(cfg.join("config.toml")).unwrap();
    assert!(config.contains("default_registry = \"mine\""));
    assert!(config.contains("[registries.mine]"));
}

#[test]
fn adding_registry_migrates_legacy_config_and_keeps_it_default() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    let old = make_catalog(tmp.path(), "old", "old-skill", "old");
    let new = make_catalog(tmp.path(), "new", "new-skill", "new");
    fs::write(
        cfg.join("config.toml"),
        format!(
            "catalog_repository = {:?}\ncatalog_ref = \"master\"\n",
            old.display().to_string()
        ),
    )
    .unwrap();

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "add", "personal"])
        .arg(&new)
        .args(["--reference", "master", "--existing-as", "company", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Default registry: company"));
    let config = fs::read_to_string(cfg.join("config.toml")).unwrap();
    assert!(config.contains("default_registry = \"company\""));
    assert!(config.contains("[registries.company]"));
    assert!(config.contains("[registries.personal]"));
}

#[test]
fn local_catalog_check_and_add_validate_skill_metadata() {
    let tmp = TempDir::new().unwrap();
    let catalog = tmp.path().join("catalog");
    fs::create_dir_all(catalog.join("skills/broken")).unwrap();
    fs::write(catalog.join("skills/broken/SKILL.md"), "plain markdown").unwrap();
    no_external_skills(&catalog);
    no_skill_groups(&catalog);
    bin()
        .args(["catalog", "check"])
        .arg(&catalog)
        .assert()
        .success()
        .stdout(predicate::str::contains("missing YAML frontmatter"));
    bin()
        .args(["catalog", "check"])
        .arg(&catalog)
        .arg("--strict")
        .assert()
        .failure()
        .stderr(predicate::str::contains("metadata validation failed"));

    let remote = tmp.path().join("remote");
    fs::create_dir_all(&remote).unwrap();
    fs::write(
        remote.join("SKILL.md"),
        "---\nname: remote\ndescription: A valid remote skill.\n---\n# Remote\n",
    )
    .unwrap();
    commit_repo(&remote);
    bin()
        .args(["catalog", "add", "remote", "--source"])
        .arg(&remote)
        .args(["--reference", "master", "--path"])
        .arg(&catalog)
        .assert()
        .success();
    assert!(fs::read_to_string(catalog.join("external-skills.toml"))
        .unwrap()
        .contains("[skills.remote]"));
}

#[cfg(unix)]
#[test]
fn registry_mutations_preserve_stow_style_config_symlink() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let dotfiles = tmp.path().join("dotfiles/config.toml");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
    fs::write(
        &dotfiles,
        "default_registry = \"old\"\n\n[registries.old]\nrepository = \"repo\"\nref = \"main\"\n",
    )
    .unwrap();
    symlink(&dotfiles, cfg.join("config.toml")).unwrap();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "rename", "old", "company"])
        .assert()
        .success();
    assert!(fs::symlink_metadata(cfg.join("config.toml"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::read_to_string(&dotfiles)
        .unwrap()
        .contains("[registries.company]"));
}

#[test]
fn built_in_skill_installs_lists_and_bulk_updates_without_a_registry() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("empty-config");
    let root = tmp.path().join("skills");

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["list", "--builtins"])
        .assert()
        .success()
        .stdout(predicate::str::contains("builtin:skilldeck"));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("builtin:skilldeck"));

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install", "--yes", "builtin:skilldeck"])
        .arg(&root)
        .assert()
        .success();
    let installed = root.join("skilldeck/SKILL.md");
    assert!(fs::read_to_string(&installed)
        .unwrap()
        .contains("Skilldeck CLI Skill"));
    assert!(
        fs::read_to_string(root.join(".skilldeck/installations.toml"))
            .unwrap()
            .contains("kind = \"built-in\"")
    );

    fs::write(&installed, "outdated").unwrap();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .arg("update")
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("built-in skill skilldeck"));
    assert!(fs::read_to_string(installed)
        .unwrap()
        .contains("Skilldeck CLI Skill"));

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "add", "builtin", "unused", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reserved"));
}

#[test]
fn persisted_local_registry_paths_are_absolute() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    make_catalog(tmp.path(), "first", "one", "one");
    make_catalog(tmp.path(), "second", "two", "two");

    bin()
        .current_dir(tmp.path())
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args([
            "init",
            "--yes",
            "--name",
            "first",
            "--repository",
            "./first",
            "--reference",
            "master",
        ])
        .assert()
        .success();
    let first_config = fs::read_to_string(cfg.join("config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&first_config).unwrap();
    let first_repository = parsed["registries"]["first"]["repository"]
        .as_str()
        .unwrap();
    assert!(Path::new(first_repository).is_absolute());
    assert_eq!(
        fs::canonicalize(first_repository).unwrap(),
        fs::canonicalize(tmp.path().join("first")).unwrap()
    );

    bin()
        .current_dir(tmp.path())
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args([
            "registry",
            "add",
            "second",
            "./second",
            "--reference",
            "master",
            "--yes",
        ])
        .assert()
        .success();
    let second_config = fs::read_to_string(cfg.join("config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&second_config).unwrap();
    let second_repository = parsed["registries"]["second"]["repository"]
        .as_str()
        .unwrap();
    assert!(Path::new(second_repository).is_absolute());
    assert_eq!(
        fs::canonicalize(second_repository).unwrap(),
        fs::canonicalize(tmp.path().join("second")).unwrap()
    );
}

#[test]
fn local_flag_reads_uncommitted_catalog_without_changing_registry_config() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let catalog = tmp.path().join("catalog");
    fs::create_dir_all(catalog.join("skills/demo")).unwrap();
    fs::write(catalog.join("skills/demo/SKILL.md"), "committed").unwrap();
    no_external_skills(&catalog);
    write_skill_groups(&catalog, vec![("test", "demo")]);
    commit_repo(&catalog);

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["init", "--yes", "--name", "coak", "--repository"])
        .arg(&catalog)
        .args(["--reference", "master"])
        .assert()
        .success();
    let config_before = fs::read(cfg.join("config.toml")).unwrap();
    fs::write(catalog.join("skills/demo/SKILL.md"), "uncommitted v2").unwrap();

    bin()
        .current_dir(&catalog)
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["list", "coak", "--local"])
        .assert()
        .success()
        .stdout(predicate::str::contains("working-tree"));
    bin()
        .current_dir(&catalog)
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["doctor", "--registry", "coak", "--local"])
        .assert()
        .success();

    let remote_root = tmp.path().join("remote-root");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install", "--yes", "coak:demo"])
        .arg(&remote_root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(remote_root.join("demo/SKILL.md")).unwrap(),
        "committed"
    );

    let local_root = tmp.path().join("local-root");
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install", "--yes", "coak:demo"])
        .arg(&local_root)
        .arg("--local")
        .arg(&catalog)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(local_root.join("demo/SKILL.md")).unwrap(),
        "uncommitted v2"
    );
    assert!(
        fs::read_to_string(local_root.join(".skilldeck/installations.toml"))
            .unwrap()
            .contains("kind = \"local-catalog\"")
    );

    let group_root = tmp.path().join("group-root");
    bin()
        .current_dir(&catalog)
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["install-group", "--yes", "coak:test"])
        .arg(&group_root)
        .arg("--local")
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(group_root.join("demo/SKILL.md")).unwrap(),
        "uncommitted v2"
    );

    fs::write(catalog.join("skills/demo/SKILL.md"), "uncommitted v3").unwrap();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .arg("update")
        .arg(&local_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("local catalog skill"));
    assert_eq!(
        fs::read_to_string(local_root.join("demo/SKILL.md")).unwrap(),
        "uncommitted v3"
    );

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["update", "coak:demo"])
        .arg(&local_root)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(local_root.join("demo/SKILL.md")).unwrap(),
        "committed"
    );
    assert_eq!(fs::read(cfg.join("config.toml")).unwrap(), config_before);
}

#[test]
fn registry_management_lifecycle_commands_and_errors() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let alpha = make_catalog(tmp.path(), "alpha-registry", "alpha", "alpha");
    let beta = make_catalog(tmp.path(), "beta-registry", "beta", "beta");
    let replacement = make_catalog(
        tmp.path(),
        "replacement-registry",
        "replacement",
        "replacement",
    );

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "add", "alpha"])
        .arg(&alpha)
        .args(["--reference", "master", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Default registry: alpha"));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_registry"));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("config.toml"));

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "add", "beta"])
        .arg(&beta)
        .args(["--reference", "master", "--default", "--yes"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "update", "beta", "--repository"])
        .arg(&replacement)
        .args(["--reference", "master"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "doctor", "beta", "--deep"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "doctor", "--all"])
        .assert()
        .success();

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "rename", "beta", "renamed"])
        .assert()
        .success();
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args([
            "registry",
            "remove",
            "renamed",
            "--new-default",
            "alpha",
            "--yes",
        ])
        .assert()
        .success();

    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "update", "alpha"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "requires --repository or --reference",
        ));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "remove", "alpha", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("only configured registry"));
    bin()
        .env("SKILLDECK_CONFIG_DIR", &cfg)
        .args(["registry", "default", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("registry not found"));
}

#[test]
fn recipe_install_renders_partials_references_and_reuses_locked_values() {
    let tmp = TempDir::new().unwrap();
    let catalog = tmp.path().join("catalog");
    let skill = catalog.join("skills/review");
    fs::create_dir_all(skill.join("references")).unwrap();
    fs::create_dir_all(catalog.join("partials/reviews")).unwrap();
    fs::write(
        skill.join("recipe.toml"),
        r#"version = 1
template = "SKILL.recipe.md"

[inputs.include_security]
type = "boolean"
default = true

[inputs.review_style]
type = "choice"
choices = ["concise", "detailed"]
default = "concise"

[inputs.ticket_system]
type = "string"
required = true
example = "Linear"

[local_inputs.model]
type = "choice"
prompt = "Default delegated-agent model"
choices = ["gpt", "claude-opus", "claude-sonnet"]
default = "claude-sonnet"
allow_invocation_override = true
"#,
    )
    .unwrap();
    fs::write(
        skill.join("SKILL.recipe.md"),
        r#"---
name: review
description: Review changes using project conventions.
---
Style: {{ review_style }}
Tickets: {{ ticket_system }}
{% if include_security %}Include security checks.{% endif %}
{% include "partials/reviews/output.recipe.md" %}
"#,
    )
    .unwrap();
    fs::write(
        skill.join("references/checklist.recipe.md"),
        "Configured for {{ ticket_system }}.\n",
    )
    .unwrap();
    fs::write(
        catalog.join("partials/reviews/output.recipe.md"),
        "Output findings in priority order.\n",
    )
    .unwrap();
    no_external_skills(&catalog);
    no_skill_groups(&catalog);

    let root = tmp.path().join("installed");
    bin()
        .env("SKILLDECK_CONFIG_DIR", tmp.path().join("cfg"))
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .args(["install", "review"])
        .arg(&root)
        .args([
            "--local",
            catalog.to_str().unwrap(),
            "--yes",
            "--set",
            "include_security=false",
            "--set",
            "model=gpt",
            "--set",
            "review_style=detailed",
            "--set",
            "ticket_system=Linear",
        ])
        .assert()
        .success();

    let installed = root.join("review");
    let body = fs::read_to_string(installed.join("SKILL.md")).unwrap();
    assert!(body.contains("Style: detailed"));
    assert!(body.contains("Tickets: Linear"));
    assert!(body.contains("Output findings in priority order."));
    assert!(!body.contains("Include security checks."));
    assert!(body.contains("## Before starting: local overrides"));
    assert!(body.contains("`model` — Default delegated-agent model."));
    assert!(body.contains("one invocation with `key=value`"));
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.local.toml")).unwrap(),
        "model = \"gpt\"\n"
    );
    assert!(
        fs::read_to_string(installed.join("SKILL.local.example.toml"))
            .unwrap()
            .contains("model = \"claude-sonnet\"")
    );
    assert!(fs::read_to_string(installed.join(".gitignore"))
        .unwrap()
        .contains("/SKILL.local.toml"));
    git(&root, &["init"]);
    assert_eq!(
        git_stdout(&root, &["check-ignore", "review/SKILL.local.toml"]),
        "review/SKILL.local.toml"
    );
    assert_eq!(
        fs::read_to_string(installed.join("references/checklist.md")).unwrap(),
        "Configured for Linear."
    );
    assert!(!installed.join("recipe.toml").exists());
    assert!(!installed.join("SKILL.recipe.md").exists());
    assert!(!installed.join("references/checklist.recipe.md").exists());

    let manifest = fs::read_to_string(root.join(".skilldeck/installations.toml")).unwrap();
    assert!(manifest.contains("format_version = 1"));
    assert!(manifest.contains("include_security = false"));
    assert!(manifest.contains("review_style = \"detailed\""));
    assert!(manifest.contains("ticket_system = \"Linear\""));
    assert!(!manifest.contains("model"));

    fs::write(
        installed.join("SKILL.local.toml"),
        "model = \"claude-opus\"\n",
    )
    .unwrap();
    fs::write(
        skill.join("SKILL.recipe.md"),
        r#"---
name: review
description: Review changes using project conventions.
---
Updated {{ review_style }} review for {{ ticket_system }}.
"#,
    )
    .unwrap();
    bin()
        .env("SKILLDECK_CONFIG_DIR", tmp.path().join("cfg"))
        .arg("update")
        .arg(&root)
        .assert()
        .success();
    assert!(fs::read_to_string(installed.join("SKILL.md"))
        .unwrap()
        .contains("Updated detailed review for Linear."));
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.local.toml")).unwrap(),
        "model = \"claude-opus\"\n"
    );
}

#[test]
fn recipe_install_prompts_for_required_input_and_set_validates_types() {
    let tmp = TempDir::new().unwrap();
    let catalog = tmp.path().join("catalog");
    let skill = catalog.join("skills/review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("recipe.toml"),
        r#"version = 1
template = "SKILL.recipe.md"

[inputs.ticket_system]
type = "string"
prompt = "Ticket system"
required = true
example = "Linear"
"#,
    )
    .unwrap();
    fs::write(
        skill.join("SKILL.recipe.md"),
        "---\nname: review\ndescription: Configured review.\n---\nUse {{ ticket_system }}.\n",
    )
    .unwrap();
    no_external_skills(&catalog);
    no_skill_groups(&catalog);
    let root = tmp.path().join("installed");
    fs::create_dir_all(&root).unwrap();

    bin()
        .env("SKILLDECK_CONFIG_DIR", tmp.path().join("cfg"))
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .args(["install", "review"])
        .arg(&root)
        .args(["--local", catalog.to_str().unwrap()])
        .write_stdin("Jira\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("Ticket system"));
    assert!(fs::read_to_string(root.join("review/SKILL.md"))
        .unwrap()
        .contains("Use Jira."));

    bin()
        .env("SKILLDECK_CONFIG_DIR", tmp.path().join("cfg"))
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .args(["install", "review"])
        .arg(tmp.path().join("other"))
        .args([
            "--local",
            catalog.to_str().unwrap(),
            "--yes",
            "--set",
            "unknown=value",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown recipe input `unknown`"));
}

#[test]
fn strict_catalog_check_reports_recipe_render_errors() {
    let tmp = TempDir::new().unwrap();
    let catalog = tmp.path().join("catalog");
    let skill = catalog.join("skills/broken");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("recipe.toml"),
        "version = 1\ntemplate = \"SKILL.recipe.md\"\n",
    )
    .unwrap();
    fs::write(
        skill.join("SKILL.recipe.md"),
        "---\nname: broken\ndescription: Broken recipe.\n---\n{{ missing_value }}\n",
    )
    .unwrap();
    no_external_skills(&catalog);
    no_skill_groups(&catalog);

    bin()
        .args(["catalog", "check"])
        .arg(&catalog)
        .arg("--strict")
        .assert()
        .failure()
        .stdout(predicate::str::contains("undefined value"));
}

#[test]
fn external_recipe_wraps_upstream_skill_and_preserves_assets() {
    let tmp = TempDir::new().unwrap();
    let external = tmp.path().join("external");
    fs::create_dir_all(external.join("skills/base")).unwrap();
    fs::write(
        external.join("skills/base/SKILL.md"),
        "---\nname: base\ndescription: Base review.\n---\nBase guidance.\n",
    )
    .unwrap();
    fs::write(external.join("skills/base/checklist.txt"), "keep me").unwrap();
    commit_repo(&external);

    let catalog = tmp.path().join("catalog");
    fs::create_dir_all(catalog.join("recipes/company-review")).unwrap();
    fs::create_dir_all(catalog.join("partials/company")).unwrap();
    fs::write(
        catalog.join("recipes/company-review/recipe.toml"),
        "version = 1\ntemplate = \"SKILL.recipe.md\"\n",
    )
    .unwrap();
    fs::write(
        catalog.join("recipes/company-review/SKILL.recipe.md"),
        r#"---
name: company-review
description: Company review guidance.
---
{% include "partials/company/policy.recipe.md" %}
{{ upstream.body }}
"#,
    )
    .unwrap();
    fs::write(
        catalog.join("partials/company/policy.recipe.md"),
        "Company policy.\n",
    )
    .unwrap();
    fs::write(
        catalog.join("external-skills.toml"),
        format!(
            "[skills.company-review]\nsource = {:?}\nsubdirectory = \"skills/base\"\nref = \"master\"\nrecipe = \"recipes/company-review/recipe.toml\"\n",
            external.display().to_string()
        ),
    )
    .unwrap();
    no_skill_groups(&catalog);

    bin()
        .args(["catalog", "check"])
        .arg(&catalog)
        .args(["--deep", "--strict"])
        .assert()
        .success()
        .stdout(predicate::str::contains("External company-review: ok"));

    let root = tmp.path().join("installed");
    bin()
        .env("SKILLDECK_CONFIG_DIR", tmp.path().join("cfg"))
        .env("SKILLDECK_CATALOG_REPOSITORY", &catalog)
        .args(["install", "company-review"])
        .arg(&root)
        .args(["--local", catalog.to_str().unwrap(), "--yes"])
        .assert()
        .success();

    let body = fs::read_to_string(root.join("company-review/SKILL.md")).unwrap();
    assert!(body.contains("Company policy."));
    assert!(body.contains("Base guidance."));
    assert_eq!(
        fs::read_to_string(root.join("company-review/checklist.txt")).unwrap(),
        "keep me"
    );
}

#[test]
fn install_defaults_to_project_agents_skills_and_rejects_ambiguous_scopes() {
    let f = Fixture::new();
    let project = f.tmp.path().join("project-default-skills");
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init"]);

    f.cmd()
        .current_dir(&project)
        .args(["install", "alpha"])
        .write_stdin("n\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains(".agents/skills"));
    assert!(!project.join(".agents/skills").exists());

    f.cmd()
        .current_dir(&project)
        .args(["install", "alpha", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".agents/skills/alpha"));
    assert!(project.join(".agents/skills/alpha/SKILL.md").is_file());
    assert!(project
        .join(".agents/skills/.skilldeck/installations.toml")
        .is_file());

    f.cmd()
        .current_dir(&project)
        .args(["install", "beta", "custom", "--global"])
        .assert()
        .failure();
    f.cmd()
        .current_dir(&project)
        .args(["install", "beta", "custom", "--claude"])
        .assert()
        .failure();

    let outside = f.tmp.path().join("not-a-project");
    fs::create_dir_all(&outside).unwrap();
    f.cmd()
        .current_dir(&outside)
        .args(["install", "beta", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("could not find a Git project"));
}

#[test]
fn native_targets_install_real_skills_and_warn_about_cross_target_name_collisions() {
    let f = Fixture::new();
    let project = f.tmp.path().join("project-native-targets");
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init"]);

    f.cmd()
        .current_dir(&project)
        .args(["install", "alpha", "--yes"])
        .assert()
        .success();
    f.cmd()
        .current_dir(&project)
        .args(["install", "alpha", "--target", "pi", "--yes"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("skill `alpha` is also installed")
                .and(predicate::str::contains(".agents/skills/alpha"))
                .and(predicate::str::contains("distinct skill name")),
        );
    assert!(project.join(".pi/skills/alpha/SKILL.md").is_file());

    f.cmd()
        .current_dir(&project)
        .args(["install", "beta", "--target", "claude", "--yes"])
        .assert()
        .success();
    let native_claude = project.join(".claude/skills/beta");
    assert!(native_claude.join("SKILL.md").is_file());
    assert!(!fs::symlink_metadata(&native_claude)
        .unwrap()
        .file_type()
        .is_symlink());
    let ignored = Command::new("git")
        .current_dir(&project)
        .args(["check-ignore", ".claude/skills/beta"])
        .output()
        .unwrap();
    assert!(!ignored.status.success());

    for (target, relative) in [
        ("codex", ".codex/skills"),
        ("gemini", ".gemini/skills"),
        ("cursor", ".cursor/skills"),
        ("opencode", ".opencode/skills"),
    ] {
        let target_project = f.tmp.path().join(format!("project-target-{target}"));
        fs::create_dir_all(&target_project).unwrap();
        git(&target_project, &["init"]);
        f.cmd()
            .current_dir(&target_project)
            .args(["install", "alpha", "--target", target, "--yes"])
            .assert()
            .success();
        assert!(target_project
            .join(relative)
            .join("alpha/SKILL.md")
            .is_file());
    }

    f.cmd()
        .current_dir(&project)
        .args(["install", "beta", "custom", "--target", "pi"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "custom install directory cannot be combined with --target",
        ));
    f.cmd()
        .current_dir(&project)
        .args(["install", "beta", "--target", "pi", "--claude"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--claude is a compatibility alias for --target agents",
        ));
}

#[cfg(unix)]
#[test]
fn claude_adapter_links_syncs_reports_and_removes_without_git_changes() {
    let f = Fixture::new();
    let project = f.tmp.path().join("project-claude-adapter");
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init"]);

    f.cmd()
        .current_dir(&project)
        .args(["install", "alpha", "--yes", "--claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Linked Claude Code alias"));

    let alpha_alias = project.join(".claude/skills/alpha");
    assert!(fs::symlink_metadata(&alpha_alias)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&alpha_alias).unwrap(),
        Path::new("../../.agents/skills/alpha")
    );
    assert_eq!(
        git_stdout(&project, &["check-ignore", ".claude/skills/alpha"]),
        ".claude/skills/alpha"
    );
    let exclude = fs::read_to_string(project.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("# BEGIN skilldeck claude aliases"));
    assert!(exclude.contains("/.claude/skills/alpha"));
    assert!(!exclude.contains("/.claude/skills/\n"));

    f.cmd()
        .current_dir(&project)
        .args(["install", "beta", "--yes"])
        .assert()
        .success();
    f.cmd()
        .current_dir(&project)
        .args(["harness", "status", "claude"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Missing links:\n  beta"));

    f.cmd()
        .current_dir(&project)
        .args(["harness", "sync", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Linked skills: 2"));
    assert!(project.join(".claude/skills/beta").is_symlink());

    f.cmd()
        .current_dir(&project)
        .args(["harness", "status", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Linked skills: 2"));

    f.cmd()
        .current_dir(&project)
        .args(["harness", "remove", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 2"));
    assert!(fs::symlink_metadata(&alpha_alias).is_err());
    assert!(fs::symlink_metadata(project.join(".claude/skills/beta")).is_err());
    assert!(!fs::read_to_string(project.join(".git/info/exclude"))
        .unwrap()
        .contains("skilldeck claude aliases"));

    fs::create_dir_all(&alpha_alias).unwrap();
    fs::write(alpha_alias.join("SKILL.md"), "native Claude skill").unwrap();
    f.cmd()
        .current_dir(&project)
        .args(["harness", "sync", "claude"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not the managed alias"));
    assert_eq!(
        fs::read_to_string(alpha_alias.join("SKILL.md")).unwrap(),
        "native Claude skill"
    );
}

#[cfg(unix)]
#[test]
fn global_and_claude_flags_stack_under_user_home() {
    let f = Fixture::new();
    let home = f.tmp.path().join("isolated-home");
    fs::create_dir_all(&home).unwrap();

    f.cmd()
        .env("HOME", &home)
        .args(["install", "alpha", "--global", "--claude", "--yes"])
        .assert()
        .success();
    assert!(home.join(".agents/skills/alpha/SKILL.md").is_file());
    assert_eq!(
        fs::read_link(home.join(".claude/skills/alpha")).unwrap(),
        Path::new("../../.agents/skills/alpha")
    );

    f.cmd()
        .env("HOME", &home)
        .args(["harness", "status", "claude", "--global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Linked skills: 1"));

    f.cmd()
        .env("HOME", &home)
        .args(["install", "beta", "--global", "--target", "pi", "--yes"])
        .assert()
        .success();
    assert!(home.join(".pi/agent/skills/beta/SKILL.md").is_file());

    f.cmd()
        .env("HOME", &home)
        .args([
            "install", "alpha", "--global", "--target", "opencode", "--yes",
        ])
        .assert()
        .success();
    assert!(home
        .join(".config/opencode/skills/alpha/SKILL.md")
        .is_file());
}
