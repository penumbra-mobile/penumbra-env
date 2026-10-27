use std::env;
use std::fs;

#[derive(Clone, Debug)]
pub struct UserAccount {
    pub username: String,
    pub display_name: String,
    pub uid: i32,
}

pub fn get_available_users() -> Vec<UserAccount> {
    let mut users = Vec::new();

    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // username:password:uid:gid:gecos:home:shell
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 7 {
                let username = parts[0];
                let uid = parts[2].parse::<i32>().unwrap_or(-1);
                let gecos = parts[4];
                let shell = parts[6];

                // Standard human users usually have UID >= 1000 and valid login shell
                if uid >= 1000 && uid < 65534 && !shell.ends_with("/nologin") && !shell.ends_with("/false") {
                    let display_name = gecos.split(',').next().unwrap_or("").trim().to_string();
                    let final_display = if display_name.is_empty() {
                        username.to_string()
                    } else {
                        display_name
                    };

                    users.push(UserAccount {
                        username: username.to_string(),
                        display_name: final_display,
                        uid,
                    });
                }
            }
        }
    }

    // If PENUMBRA_LOGIN_USER is specified, ensure it is prioritized or present
    if let Ok(default_user) = env::var("PENUMBRA_LOGIN_USER") {
        if !default_user.trim().is_empty() {
            let default_user = default_user.trim();
            if let Some(pos) = users.iter().position(|u| u.username == default_user) {
                let u = users.remove(pos);
                users.insert(0, u);
            } else {
                users.insert(
                    0,
                    UserAccount {
                        username: default_user.to_string(),
                        display_name: default_user.to_string(),
                        uid: 1000,
                    },
                );
            }
        }
    }

    if users.is_empty() {
        if let Ok(u) = env::var("USER") {
            if !u.trim().is_empty() {
                users.push(UserAccount {
                    username: u.clone(),
                    display_name: u,
                    uid: unsafe { libc::getuid() as i32 },
                });
            }
        }
    }

    users
}
