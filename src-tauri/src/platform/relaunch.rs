use std::path::Path;

pub fn after_update(app: &tauri::AppHandle, executable: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::{
            fs::OpenOptions,
            os::unix::process::CommandExt,
            process::{Command, Stdio},
        };
        use tauri::Manager;

        let bundle = bundle_path(executable).ok_or(
            "The updated application bundle could not be located. Quit and reopen GateTree-Ops.",
        )?;
        let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("update-relaunch.log"))
            .map_err(|e| e.to_string())?;
        Command::new("/bin/sh")
            .arg("-c").arg(include_str!("relaunch-macos.sh"))
            .arg("gatetree-ops-relaunch")
            .arg(std::process::id().to_string())
            .arg(bundle)
            .arg("/usr/bin/open")
            .arg("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
            .arg("300")
            .args(std::env::args_os().skip(1))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(log)
            .process_group(0)
            .spawn()
            .map_err(|e| format!("Update installed, but relaunch could not be prepared: {e}. Quit and reopen GateTree-Ops."))?;
        app.exit(0);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = executable;
        app.restart();
    }
}

#[cfg(any(target_os = "macos", test))]
fn bundle_path(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then_some(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunch_uses_the_installed_bundle_instead_of_a_binary_or_parent_bundle() {
        assert_eq!(
            bundle_path(Path::new(
                "/Applications/GateTree-Ops.app/Contents/MacOS/gatetree-ops"
            )),
            Some(Path::new("/Applications/GateTree-Ops.app"))
        );
        assert!(
            bundle_path(Path::new("/tmp/GateTree-Ops.app/target/debug/gatetree-ops")).is_none()
        );
        assert!(bundle_path(Path::new("/tmp/Contents/MacOS/gatetree-ops")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn relaunch_waits_for_exit_and_preserves_bundle_and_argument_boundaries() {
        use std::{
            os::unix::fs::PermissionsExt,
            process::{Command, Stdio},
            time::Duration,
        };
        let root = std::env::temp_dir().join(format!("gatetree-relaunch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let output = root.join("calls");
        let tool = root.join("launch tool");
        std::fs::write(
            &tool,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$RELAUNCH_TEST_OUTPUT\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut previous = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let bundle = root.join("GateTree ' $literal.app");
        let mut helper = Command::new("/bin/sh")
            .arg("-c")
            .arg(include_str!("relaunch-macos.sh"))
            .arg("relaunch-test")
            .arg(previous.id().to_string())
            .arg(&bundle)
            .arg(&tool)
            .arg(&tool)
            .arg("100")
            .arg("workspace with spaces")
            .env("RELAUNCH_TEST_OUTPUT", &output)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !output.exists(),
            "must not launch while the old process is alive"
        );
        previous.kill().unwrap();
        previous.wait().unwrap();
        assert!(helper.wait().unwrap().success());
        let calls = std::fs::read_to_string(&output).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            vec![
                "-f",
                bundle.to_str().unwrap(),
                "-a",
                bundle.to_str().unwrap(),
                "--args",
                "workspace with spaces"
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn relaunch_does_not_open_another_instance_if_the_old_process_cannot_exit() {
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(include_str!("relaunch-macos.sh"))
            .arg("relaunch-test")
            .arg(std::process::id().to_string())
            .arg("/Applications/GateTree-Ops.app")
            .arg("/must-not-run-opener")
            .arg("/must-not-run-registrar")
            .arg("1")
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(1));
    }
}
