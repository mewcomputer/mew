use serde_yaml::Value;

fn workflow(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows")
        .join(name);
    serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn rust_jobs_install_the_pinned_ghostty_toolchain_before_building() {
    for name in ["ci.yml", "nightly.yml", "release.yml"] {
        let workflow = workflow(name);
        for (job_name, job) in workflow["jobs"].as_mapping().unwrap() {
            let steps = job["steps"].as_sequence().unwrap();
            let Some(build) = steps.iter().position(|step| {
                step["run"]
                    .as_str()
                    .is_some_and(|run| run.contains("cargo build") || run.contains("cargo clippy"))
            }) else {
                continue;
            };
            assert!(
                steps[..build].iter().any(|step| {
                    step["uses"]
                        .as_str()
                        .is_some_and(|action| action.starts_with("mlugg/setup-zig@"))
                        && step["with"]["version"].as_str() == Some("0.15.2")
                }),
                "{name} {} requires Zig before compiling Ghostty",
                job_name.as_str().unwrap()
            );
        }
    }
}

#[test]
fn published_binaries_build_on_matching_supported_native_runners() {
    for name in ["nightly.yml", "release.yml"] {
        let workflow = workflow(name);
        for (job_name, job) in workflow["jobs"].as_mapping().unwrap() {
            let Some(matrix) = job["strategy"]["matrix"]["include"].as_sequence() else {
                continue;
            };
            for entry in matrix {
                let target = entry["target"].as_str().unwrap();
                let runner = entry["os"].as_str().unwrap();
                let expected = match target {
                    "aarch64-apple-darwin" => "macos-15",
                    "x86_64-apple-darwin" => "macos-15-intel",
                    "aarch64-unknown-linux-gnu" => "ubuntu-24.04-arm",
                    "x86_64-unknown-linux-gnu" => "ubuntu-24.04",
                    other => panic!("unverified release target {other}"),
                };
                assert_eq!(
                    runner,
                    expected,
                    "{name} {} {target}",
                    job_name.as_str().unwrap()
                );
                assert!(entry["cross_linker"].is_null());
                assert!(entry["cross_pkg"].is_null());
            }
        }
    }
}

#[test]
fn web_ci_uses_a_runtime_that_executes_the_typescript_tests() {
    let workflow = workflow("ci.yml");
    for job in workflow["jobs"].as_mapping().unwrap().values() {
        for step in job["steps"].as_sequence().unwrap() {
            if step["uses"]
                .as_str()
                .is_some_and(|action| action.starts_with("actions/setup-node@"))
            {
                assert_eq!(step["with"]["node-version"].as_u64(), Some(24));
            }
        }
    }
    let package: serde_json::Value =
        serde_json::from_str(include_str!("../../../package.json")).unwrap();
    assert_eq!(package["engines"]["node"], ">=24");
}

#[test]
fn web_ci_builds_the_site_that_loads_repository_documentation() {
    let workflow = workflow("ci.yml");
    assert!(workflow["jobs"]["web-ci"]["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|step| step["run"]
            .as_str()
            .is_some_and(|run| run.contains("pnpm --filter site build"))));
}

#[test]
fn desktop_release_installs_and_checks_the_browser_before_packaging() {
    let workflow = workflow("release.yml");
    let job = &workflow["jobs"]["desktop-build"];
    let steps = job["steps"].as_sequence().unwrap();
    let package = steps
        .iter()
        .position(|step| {
            step["run"]
                .as_str()
                .is_some_and(|run| run.contains("scripts/package-desktop-native.sh"))
        })
        .unwrap();
    assert!(steps[..package].iter().any(|step| {
        step["run"].as_str().is_some_and(|run| {
            run.contains("agent-browser")
                && run.contains("shasum -a 256 -c")
                && run.contains("--version")
                && run.contains("GITHUB_PATH")
        })
    }));
    for entry in job["strategy"]["matrix"]["include"].as_sequence().unwrap() {
        assert_eq!(entry["browser_sha256"].as_str().unwrap().len(), 64);
        let expected_arch = if entry["target"] == "aarch64-apple-darwin" {
            "arm64"
        } else {
            "x64"
        };
        assert_eq!(entry["browser_arch"].as_str(), Some(expected_arch));
    }
}

#[test]
fn linux_builds_install_native_terminal_and_desktop_link_dependencies() {
    for name in ["ci.yml", "nightly.yml", "release.yml"] {
        let workflow = workflow(name);
        let job = if name == "ci.yml" { "rust-ci" } else { "build" };
        let steps = workflow["jobs"][job]["steps"].as_sequence().unwrap();
        let install = steps
            .iter()
            .find_map(|step| {
                (step["if"].as_str() == Some("runner.os == 'Linux'"))
                    .then(|| step["run"].as_str())
                    .flatten()
            })
            .unwrap();
        for package in ["libssl-dev", "pkg-config", "libc++-dev"] {
            assert!(
                install.split_whitespace().any(|word| word == package),
                "{name}: {package}"
            );
        }
        if name == "ci.yml" {
            for package in [
                "cmake",
                "ninja-build",
                "libfontconfig1-dev",
                "libwayland-dev",
                "libxkbcommon-x11-dev",
                "libgtk-3-dev",
                "libnss3-dev",
            ] {
                assert!(
                    install.split_whitespace().any(|word| word == package),
                    "{name}: {package}"
                );
            }
        }
    }
}
