//! Process-level check that the production shutdown signal path handles SIGTERM.

#![cfg(unix)]

use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use llm_d_sc::grpc::classify::ClassifyServer;

const CHILD_MODE: &str = "LLM_D_SC_SIGTERM_TEST_CHILD";
const LISTEN_ADDR: &str = "LLM_D_SC_SIGTERM_TEST_ADDR";
const TEST_NAME: &str = "i013_sigterm_is_handled_and_server_exits_after_drain";

#[test]
fn i013_sigterm_is_handled_and_server_exits_after_drain() {
    if std::env::var_os(CHILD_MODE).is_some() {
        let addr = std::env::var(LISTEN_ADDR).expect("parent supplies listen address");
        let server = ClassifyServer::bind(addr).expect("test server must bind");
        let report = server
            .wait_for_shutdown_signal(Duration::from_secs(2))
            .expect("SIGTERM must start graceful shutdown");
        assert!(report.completed(), "test server did not drain: {report:?}");
        return;
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a test port");
    let addr = listener.local_addr().expect("reserved address");
    drop(listener);

    let executable = std::env::current_exe().expect("test executable path");
    let mut child = Command::new(executable)
        .arg("--exact")
        .arg(TEST_NAME)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_MODE, "1")
        .env(LISTEN_ADDR, addr.to_string())
        .spawn()
        .expect("child test process must start");

    let ready_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(20)).is_ok() {
            break;
        }
        if let Some(status) = child.try_wait().expect("poll child startup") {
            panic!("child exited before binding the listener: {status}");
        }
        assert!(
            Instant::now() < ready_deadline,
            "child listener did not become ready"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let signal = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("kill command must send SIGTERM");
    assert!(signal.success(), "SIGTERM command failed: {signal}");

    let exit_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            assert!(status.success(), "server process exited with {status}");
            break;
        }
        if Instant::now() >= exit_deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("server process did not exit after SIGTERM");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
