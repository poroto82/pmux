use std::thread;
use std::time::Duration;

use pworkspaces::session::{Session, SessionRegistry, SessionState};

fn wait_for_output(session: &Session, timeout_ms: u64) -> Vec<u8> {
    let start = std::time::Instant::now();
    loop {
        let out = session.peek_output();
        if !out.is_empty() {
            return session.read_output();
        }
        if start.elapsed().as_millis() > timeout_ms as u128 {
            return vec![];
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_output_reg(reg: &SessionRegistry, id: &pworkspaces::ids::SessionId, timeout_ms: u64) -> Vec<u8> {
    let start = std::time::Instant::now();
    loop {
        let session = reg.get(id).unwrap();
        if session.output_len() > 0 {
            return reg.read_output(id).unwrap();
        }
        if start.elapsed().as_millis() > timeout_ms as u128 {
            return vec![];
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn spawn_session() {
    let session = Session::spawn(None, 80, 24).unwrap();
    assert_eq!(session.meta.state, SessionState::Running);
    assert_eq!(session.meta.cols, 80);
    assert_eq!(session.meta.rows, 24);
}

#[test]
fn spawn_with_pw_env() {
    use pworkspaces::session::SessionSpawnEnv;

    let env = SessionSpawnEnv::new()
        .insert("PW_WORKSPACE_NAME", "backend")
        .insert("PW_PANE_NAME", "claude");
    let mut session = Session::spawn_with_env(None, 80, 24, env).unwrap();

    wait_for_output(&session, 2000);
    session.read_output();

    session
        .send_command("echo $PW_WORKSPACE_NAME/$PW_PANE_NAME")
        .unwrap();

    let mut last = String::new();
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < 3000 {
        let out = session.read_output();
        last.push_str(&String::from_utf8_lossy(&out));
        if last.contains("backend/claude") {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("expected PW_* env in shell, got: {}", last);
}

#[test]
fn send_command_and_read_output() {
    let mut session = Session::spawn(None, 80, 24).unwrap();

    // Wait for shell prompt
    wait_for_output(&session, 2000);
    session.read_output(); // drain prompt

    session.send_command("echo HELLO_PWORKSPACES").unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = session.read_output();
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("HELLO_PWORKSPACES"),
        "expected HELLO_PWORKSPACES in output, got: {}",
        text
    );
}

#[test]
fn send_input_raw() {
    let mut session = Session::spawn(None, 80, 24).unwrap();

    // Wait for shell
    wait_for_output(&session, 2000);
    session.read_output();

    // Send raw: "echo HI\n"
    session.send_input(b"echo HI\n").unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = session.read_output();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("HI"), "got: {}", text);
}

#[test]
fn read_output_drains() {
    let mut session = Session::spawn(None, 80, 24).unwrap();

    wait_for_output(&session, 2000);
    session.read_output();

    session.send_command("echo DRAIN_TEST").unwrap();
    thread::sleep(Duration::from_millis(500));

    let first = session.read_output();
    assert!(!first.is_empty());

    // Second read should be empty (drained)
    let second = session.read_output();
    assert!(second.is_empty());
}

#[test]
fn peek_does_not_drain() {
    let mut session = Session::spawn(None, 80, 24).unwrap();

    wait_for_output(&session, 2000);
    session.read_output();

    session.send_command("echo PEEK_TEST").unwrap();
    thread::sleep(Duration::from_millis(500));

    let peek1 = session.peek_output();
    let peek2 = session.peek_output();
    assert_eq!(peek1, peek2);
    assert!(!peek1.is_empty());
}

#[test]
fn replay_survives_drain() {
    let mut session = Session::spawn(None, 80, 24).unwrap();
    wait_for_output(&session, 2000);
    session.send_command("echo REPLAY_KEEP").unwrap();
    thread::sleep(Duration::from_millis(500));

    let drained = session.read_output();
    assert!(!drained.is_empty());
    assert!(session.read_output().is_empty());

    let replay = session.replay_output();
    let text = String::from_utf8_lossy(&replay);
    assert!(text.contains("REPLAY_KEEP"), "got: {text}");
}

#[test]
fn strip_zsh_eol_mark_line() {
    let raw = b"ls\nfile\n%\n~/dev \xE2\x9D\xAF\n";
    let out = pworkspaces::session::strip_zsh_eol_marks(raw);
    let s = String::from_utf8_lossy(&out);
    assert!(!s.lines().any(|l| l.trim() == "%"), "got {s:?}");
    assert!(s.contains("ls"), "got {s:?}");
}

#[test]
fn trim_replay_skips_partial_first_line() {
    let raw = b"ial\nprompt %\nls\nfile\n";
    let trimmed = pworkspaces::session::trim_replay(raw);
    let s = String::from_utf8_lossy(&trimmed);
    assert!(!s.starts_with("ial"), "got {s:?}");
    assert!(s.contains("ls"), "got {s:?}");
}

#[test]
fn resize_session() {
    let mut session = Session::spawn(None, 80, 24).unwrap();
    assert!(session.resize(120, 40).is_ok());
}

#[test]
fn session_with_cwd() {
    let session = Session::spawn(Some("/tmp"), 80, 24).unwrap();
    assert_eq!(session.meta.cwd, "/tmp");
    assert!(session.pid().is_some());
}

// --- SessionRegistry tests ---

#[test]
fn registry_create() {
    let mut reg = SessionRegistry::new();
    let id = reg.create(None, 80, 24).unwrap();

    assert_eq!(reg.count(), 1);
    assert!(reg.get(&id).is_some());
}

#[test]
fn registry_send_command() {
    let mut reg = SessionRegistry::new();
    let id = reg.create(None, 80, 24).unwrap();

    // Wait for shell
    wait_for_output_reg(&reg, &id, 2000);
    reg.read_output(&id).unwrap();

    reg.send_command(&id, "echo REG_TEST").unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = reg.read_output(&id).unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("REG_TEST"), "got: {}", text);
}

#[test]
fn registry_destroy() {
    let mut reg = SessionRegistry::new();
    let id = reg.create(None, 80, 24).unwrap();

    assert!(reg.destroy(&id));
    assert_eq!(reg.count(), 0);
    assert!(reg.get(&id).is_none());
}

#[test]
fn registry_multiple_sessions() {
    let mut reg = SessionRegistry::new();
    let id1 = reg.create(None, 80, 24).unwrap();
    let id2 = reg.create(None, 80, 24).unwrap();

    assert_eq!(reg.count(), 2);
    assert_ne!(id1, id2);
}

#[test]
fn registry_send_to_nonexistent() {
    let mut reg = SessionRegistry::new();
    let fake = pworkspaces::ids::SessionId::new();

    let result = reg.send_command(&fake, "test");
    assert!(result.is_err());
}

#[test]
fn registry_list_meta() {
    let mut reg = SessionRegistry::new();
    reg.create(None, 80, 24).unwrap();
    reg.create(Some("/tmp"), 120, 40).unwrap();

    let metas = reg.list_meta();
    assert_eq!(metas.len(), 2);
}

#[test]
fn session_id_stable() {
    let session = Session::spawn(None, 80, 24).unwrap();
    let id1 = session.id().clone();
    let id2 = session.id().clone();
    assert_eq!(id1, id2);
}
