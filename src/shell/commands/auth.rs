use core::str;
use crate::sched;
use crate::security;
use crate::shell::init::{
    print, print_char, print_fmt, with_shell_state_mut, MAX_INPUT_LEN, PROMPT,
};

pub(crate) fn handle_login_line(line: &str, raw_line: &[u8]) {
    let stage = with_shell_state_mut(|state| state.login_stage);
    match stage {
        1 => {
            if line.is_empty() || line.len() > MAX_INPUT_LEN {
                print("login: ");
                return;
            }
            with_shell_state_mut(|state| {
                state.login_username.fill(0);
                state.login_username[..line.len()].copy_from_slice(line.as_bytes());
                state.login_stage = 2;
                state.clear_input();
                state.rendered_len = 10;
            });
            print("password: ");
        }
        2 => {
            let (username, username_len) = with_shell_state_mut(|state| {
                let length = state
                    .login_username
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(MAX_INPUT_LEN);
                let mut username = [0u8; MAX_INPUT_LEN];
                username[..length].copy_from_slice(&state.login_username[..length]);
                (username, length)
            });
            let authenticated =
                unsafe { security::login_current(&username[..username_len], raw_line) };
            with_shell_state_mut(|state| {
                state.input.fill(0);
                state.login_username.fill(0);
                state.clear_input();
            });
            if authenticated {
                with_shell_state_mut(|state| {
                    state.login_stage = 0;
                    state.rendered_len = PROMPT.len();
                });
                print("\n");
                print(PROMPT);
            } else {
                with_shell_state_mut(|state| {
                    state.login_stage = 1;
                    state.rendered_len = 7;
                });
                print("Login incorrect\nlogin: ");
            }
        }
        3 => {
            let username = with_shell_state_mut(|state| {
                let length = state
                    .pending_username
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(MAX_INPUT_LEN);
                let mut username = [0u8; MAX_INPUT_LEN];
                username[..length].copy_from_slice(&state.pending_username[..length]);
                username
            });
            let username_length = username
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(MAX_INPUT_LEN);
            let uname = &username[..username_length];
            let is_root = uname == b"root";
            let user_id = if is_root {
                0
            } else {
                security::get_account_uid(uname)
                    .unwrap_or_else(|| security::next_user_id().unwrap_or(1000))
            };
            let group_id = user_id;
            let account = security::create_account_record(
                uname,
                user_id,
                group_id,
                raw_line,
                100_000,
            );
            let installed = account
                .map(|account| security::install_or_update_account(account))
                .unwrap_or(false);
            with_shell_state_mut(|state| {
                state.input.fill(0);
                state.pending_username.fill(0);
                state.clear_input();
                state.login_stage = 0;
                state.rendered_len = PROMPT.len();
            });
            if installed {
                if unsafe { security::persist_accounts() } {
                    print("User updated/added\n");
                } else {
                    print("User updated/added for this session; persistence failed\n");
                }
            } else {
                print("User creation failed\n");
            }
            print(PROMPT);
        }
        _ => {}
    }
}

pub(crate) fn command_useradd(mut parts: str::SplitWhitespace<'_>) {
    let Some(username) = parts.next() else {
        print("usage: useradd <name>\n");
        return;
    };
    let credentials = unsafe {
        sched::current_cred()
            .as_ref()
            .copied()
            .unwrap_or_else(crate::sched::Credentials::root)
    };
    if security::check(
        &credentials,
        &security::SecurityObject::System,
        security::Operation::AccountAdmin,
    ) == security::Decision::Deny
    {
        print("permission denied\n");
        return;
    }
    if username.is_empty() || username.len() >= MAX_INPUT_LEN {
        print("invalid username\n");
        return;
    }

    with_shell_state_mut(|state| {
        state.pending_username.fill(0);
        state.pending_username[..username.len()].copy_from_slice(username.as_bytes());
        state.login_stage = 3;
        state.clear_input();
        state.rendered_len = 14;
    });
    print("New password: ");
}

pub(crate) fn command_su(mut parts: str::SplitWhitespace<'_>) {
    let target = parts.next().unwrap_or("root");
    if target.is_empty() || target.len() > MAX_INPUT_LEN {
        print("invalid username\n");
        return;
    }

    let credentials = unsafe {
        sched::current_cred()
            .as_ref()
            .copied()
            .unwrap_or_else(crate::sched::Credentials::root)
    };

    if credentials.is_root() {
        if target == "root" {
            return;
        }
        if let Some(target_cred) = security::credentials_for_user(target.as_bytes()) {
            unsafe {
                security::set_current_credentials(target_cred);
            }
            return;
        } else {
            print("user not found\n");
            return;
        }
    }

    with_shell_state_mut(|state| {
        state.login_username.fill(0);
        state.login_username[..target.len()].copy_from_slice(target.as_bytes());
        state.login_stage = 2;
        state.clear_input();
        state.rendered_len = 10;
    });
    print("password: ");
}

pub(crate) fn command_login(mut parts: str::SplitWhitespace<'_>) {
    if let Some(target) = parts.next() {
        if target.is_empty() || target.len() > MAX_INPUT_LEN {
            print("invalid username\n");
            return;
        }
        with_shell_state_mut(|state| {
            state.login_username.fill(0);
            state.login_username[..target.len()].copy_from_slice(target.as_bytes());
            state.login_stage = 2;
            state.clear_input();
            state.rendered_len = 10;
        });
        print("password: ");
    } else {
        with_shell_state_mut(|state| {
            state.login_stage = 1;
            state.clear_input();
            state.rendered_len = 7;
        });
        print("login: ");
    }
}

pub(crate) fn command_passwd(mut parts: str::SplitWhitespace<'_>) {
    let credentials = unsafe {
        sched::current_cred()
            .as_ref()
            .copied()
            .unwrap_or_else(crate::sched::Credentials::root)
    };
    let mut current_name_buf = [0u8; 32];
    let current_name_len = security::username_for_uid(credentials.uid, &mut current_name_buf).unwrap_or(0);
    let current_name = core::str::from_utf8(&current_name_buf[..current_name_len]).unwrap_or("root");

    let target = parts.next().unwrap_or(current_name);
    if !credentials.is_root() && target != current_name {
        print("permission denied\n");
        return;
    }
    if target.is_empty() || target.len() >= MAX_INPUT_LEN {
        print("invalid username\n");
        return;
    }

    with_shell_state_mut(|state| {
        state.pending_username.fill(0);
        state.pending_username[..target.len()].copy_from_slice(target.as_bytes());
        state.login_stage = 3;
        state.clear_input();
        state.rendered_len = 14;
    });
    print("New password: ");
}

pub(crate) fn command_whoami() {
    let credentials = unsafe {
        sched::current_cred()
            .as_ref()
            .copied()
            .unwrap_or_else(crate::sched::Credentials::root)
    };
    let mut name_buf = [0u8; 32];
    if let Some(len) = security::username_for_uid(credentials.uid, &mut name_buf) {
        if let Ok(name) = core::str::from_utf8(&name_buf[..len]) {
            print(name);
            print_char('\n');
            return;
        }
    }
    if credentials.is_root() {
        print("root\n");
    } else {
        print_fmt(&format_args!("uid {}\n", credentials.uid));
    }
}

pub(crate) fn command_logout() {
    with_shell_state_mut(|state| {
        state.login_stage = 1;
        state.clear_input();
        state.rendered_len = 7;
    });
    print("login: ");
}
