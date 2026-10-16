use std::{
    io::Write,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct KeePassCredential {
    pub username: String,
    pub password: String,
}

pub fn validate_keepass_database(database: &str, master_password: &str) -> Result<(), String> {
    let executable = if cfg!(target_os = "macos") {
        "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-cli"
    } else {
        "keepassxc-cli"
    };
    let output = run_cli(executable, &["ls", "--quiet", database], master_password)?;
    if output.status.success() {
        Ok(())
    } else {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if details.is_empty() {
            "Could not unlock the KeePass database.".into()
        } else {
            format!("Could not unlock the KeePass database: {details}")
        })
    }
}

pub fn read_keepass_entry(
    database: &str,
    entry_path: &str,
    master_password: &str,
) -> Result<KeePassCredential, String> {
    let candidates = entry_path_candidates(entry_path);
    let executable = if cfg!(target_os = "macos") {
        "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-cli"
    } else {
        "keepassxc-cli"
    };

    let mut failures = Vec::new();
    for candidate in &candidates {
        let output = run_cli(
            executable,
            &[
                "show",
                "--show-protected",
                "--attributes",
                "UserName",
                "--attributes",
                "Password",
                database,
                candidate,
            ],
            master_password,
        )?;
        if let Some(credential) = credential_from_output(&output) {
            return Ok(credential);
        }
        failures.push(format_failure(candidate, &output));
    }

    // KeePassXC databases can expose their root group differently through the
    // UI and CLI. If direct path lookup failed, resolve the entry
    // title only when the database contains one unambiguous match. The saved
    // Vault path remains untouched and continues to be the primary lookup key.
    if let Some(title) = entry_title(entry_path) {
        let search = run_cli(
            executable,
            &["search", "--quiet", database, title],
            master_password,
        )?;
        if search.status.success() {
            let matches = matching_search_paths(&search.stdout, title);
            if matches.len() == 1 && !candidates.contains(&matches[0]) {
                let resolved = &matches[0];
                let output = run_cli(
                    executable,
                    &[
                        "show",
                        "--show-protected",
                        "--attributes",
                        "UserName",
                        "--attributes",
                        "Password",
                        database,
                        resolved,
                    ],
                    master_password,
                )?;
                if let Some(credential) = credential_from_output(&output) {
                    return Ok(credential);
                }
                failures.push(format_failure(resolved, &output));
            } else if matches.len() > 1 {
                failures.push(format!(
                    "entry title '{title}' is ambiguous ({} matches)",
                    matches.len()
                ));
            }
        } else {
            failures.push(format_failure("entry search", &search));
        }
    }

    let details = failures.join("; ");
    Err(if details.is_empty() {
        format!("Could not read the KeePassXC entry at '{entry_path}'.")
    } else {
        format!("Could not read the KeePassXC entry at '{entry_path}'. {details}")
    })
}

fn entry_path_candidates(entry_path: &str) -> Vec<String> {
    let exact = entry_path.trim().to_owned();
    let mut candidates = vec![exact.clone()];
    let components: Vec<_> = exact
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    if components.len() > 1 {
        let root_relative = components[1..].join("/");
        if root_relative != exact {
            candidates.push(root_relative);
        }
    }
    candidates
}

fn entry_title(entry_path: &str) -> Option<&str> {
    entry_path
        .split('/')
        .rev()
        .find(|component| !component.is_empty())
}

fn matching_search_paths(output: &[u8], title: &str) -> Vec<String> {
    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(output).lines() {
        let path = line.trim();
        let matches_title =
            entry_title(path).is_some_and(|candidate| candidate.eq_ignore_ascii_case(title));
        if matches_title && !matches.iter().any(|candidate| candidate == path) {
            matches.push(path.to_owned());
        }
    }
    matches
}

fn credential_from_output(output: &Output) -> Option<KeePassCredential> {
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.split('\n');
    let username = lines.next()?.trim_end_matches('\r').to_owned();
    let password = lines.next()?.trim_end_matches('\r').to_owned();
    Some(KeePassCredential { username, password })
}

fn format_failure(candidate: &str, output: &Output) -> String {
    let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if details.is_empty() {
        format!("'{candidate}' was not found")
    } else {
        format!("'{candidate}': {details}")
    }
}

fn run_cli(executable: &str, arguments: &[&str], master_password: &str) -> Result<Output, String> {
    let mut child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("KeePassXC CLI is unavailable: {error}"))?;
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(format!("{master_password}\n").as_bytes())
            .map_err(|error| format!("Could not unlock KeePass database: {error}"))?;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("KeePassXC did not finish within 15 seconds. Check the database password, key file requirements, and entry path.".into());
            }
            Err(error) => return Err(format!("KeePassXC CLI failed: {error}")),
        }
    }
    child
        .wait_with_output()
        .map_err(|error| format!("KeePassXC CLI failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{entry_path_candidates, matching_search_paths};

    #[test]
    fn tries_saved_path_before_root_group_fallback() {
        assert_eq!(
            entry_path_candidates("/Personal/Example/Operations/account_issue"),
            vec![
                "/Personal/Example/Operations/account_issue",
                "Example/Operations/account_issue"
            ]
        );
    }

    #[test]
    fn accepts_only_unambiguous_exact_title_search_results() {
        let output = b"Personal/Example/Operations/account_issue\nPersonal/Other/account\n";
        assert_eq!(
            matching_search_paths(output, "account_issue"),
            vec!["Personal/Example/Operations/account_issue"]
        );
    }
}
