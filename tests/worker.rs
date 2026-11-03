use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

struct ServerProcess {
    child: Option<Child>,
    stdin: ChildStdin,
    stdout: Receiver<String>,
    log: PathBuf,
}

impl ServerProcess {
    fn start(project: &std::path::Path, db_dir: &std::path::Path, log: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_memlite"))
            .arg("serve")
            .arg("--project-path")
            .arg(project)
            .arg("--db-dir")
            .arg(db_dir)
            .arg("--log-file")
            .arg(log)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child: Some(child),
            stdin,
            stdout: rx,
            log: log.to_path_buf(),
        }
    }

    fn initialize(&mut self) {
        self.initialize_with_capabilities(json!({}));
    }

    fn initialize_with_capabilities(&mut self, capabilities: Value) {
        let response = self.request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": capabilities,
                "clientInfo": { "name": "worker-test", "version": "1" }
            }
        }));
        assert!(response.get("result").is_some(), "{response}");
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
        let response = self.request(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }));
        assert!(response.get("result").is_some(), "{response}");
    }

    fn request(&mut self, request: Value) -> Value {
        let id = request["id"].clone();
        self.send(request);
        loop {
            let line = self
                .stdout
                .recv_timeout(Duration::from_secs(15))
                .unwrap_or_else(|error| {
                    let log = fs::read_to_string(&self.log).unwrap_or_default();
                    panic!("timed out waiting for MCP response {id}: {error}\n{log}")
                });
            let response: Value = serde_json::from_str(&line).unwrap();
            assert!(
                response.get("method").is_none(),
                "unexpected server request: {response}"
            );
            if response.get("id") == Some(&id) {
                return response;
            }
        }
    }

    fn send(&mut self, message: Value) {
        serde_json::to_writer(&mut self.stdin, &message).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn explicit_project_does_not_request_roots() {
    let project = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let log = db_dir.path().join("explicit.log");
    let mut server = ServerProcess::start(project.path(), db_dir.path(), &log);
    server.initialize_with_capabilities(json!({"roots": {"listChanged": true}}));
}

#[test]
fn proxies_share_worker_and_promote_after_worker_exit() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let log_a = db_dir.path().join("a.log");
    let log_b = db_dir.path().join("b.log");

    let mut first = ServerProcess::start(project_a.path(), db_dir.path(), &log_a);
    first.initialize();
    let mut second = ServerProcess::start(project_b.path(), db_dir.path(), &log_b);
    second.initialize();

    let first_log = fs::read_to_string(&log_a).unwrap();
    let second_log = fs::read_to_string(&log_b).unwrap();
    assert_eq!(first_log.matches("shared worker elected").count(), 1);
    assert_eq!(first_log.matches("shared project opened").count(), 2);
    assert_eq!(second_log.matches("shared worker elected").count(), 0);
    assert_eq!(second_log.matches("shared project opened").count(), 0);

    first.kill();
    let mut succeeded = false;
    for id in 10..15 {
        let response = second.request(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": "memory_stats", "arguments": {} }
        }));
        if response.get("result").is_some() {
            succeeded = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(succeeded, "proxy never recovered after worker exit");

    let second_log = fs::read_to_string(&log_b).unwrap();
    assert_eq!(second_log.matches("shared worker elected").count(), 1);
    let database_count = fs::read_dir(db_dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "db")
        })
        .count();
    assert_eq!(database_count, 0, "read-only sessions created databases");
}
