//! Server mode end to end against a real sshd (dev only, `--ignored`):
//! `scratchpad/e2e/sshd-e2e.sh` prepares the server and runs this test.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use smt5_core::remote::{classify_ssh_error, exec_cmd, host_keys, keyscan_cmd, SshError, SshTarget};
use smt5_core::remote_setup::{parse_preflight, problems, runtime_tarball, ServerMode, PREFLIGHT_SH};
use smt5_core::{Runner, RuntimeDir, SystemRunner};

use crate::backend::{server_job_plan, AppConfig, JobStep};
use crate::servers::{new_server, remember_host_key, ssh_opts};
use crate::targets::{api_with, read_secrets, Link, Targets};

fn run_steps(steps: Vec<JobStep>) {
    for step in steps {
        let JobStep::Run(cmd) = step else { panic!("unexpected step {step:?}") };
        let mut out = Vec::new();
        let code = smt5_core::exec::stream(&cmd, |l| out.push(l.to_owned()), |_| {});
        assert_eq!(code, 0, "step failed:\n{}", out.join("\n"));
    }
}

#[test]
#[ignore]
fn e2e_server_mode() {
    let (Ok(ssh), Ok(key)) = (std::env::var("SMT5_E2E_SSH"), std::env::var("SMT5_E2E_KEY")) else {
        panic!("run through scratchpad/e2e/sshd-e2e.sh");
    };
    let dir = std::env::temp_dir().join(format!("smt5-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = RuntimeDir::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../runtime"));

    // probe + accept the host key
    let t = SshTarget::parse(&ssh).unwrap();
    let scan = SystemRunner.run(&keyscan_cmd(&t.host, t.port.unwrap()));
    let keys = host_keys(&scan.stdout, &SystemRunner);
    let ed = keys.iter().find(|k| k.kind == "ED25519").expect("ed25519 host key");
    let mut cfg = AppConfig::default();
    let s = new_server(&cfg, "e2e", &ssh, Some(PathBuf::from(&key)), &ed.sha256).unwrap();
    cfg.servers.push(s.clone());
    let opts = ssh_opts(&dir, &s);
    remember_host_key(&opts, &keys, &ed.sha256).unwrap();
    println!("ok    host key {}", ed.sha256);

    // key login, preflight
    assert!(SystemRunner.run(&exec_cmd(&t, &opts, "true")).ok(), "BatchMode key login");
    let pf = parse_preflight(&SystemRunner.run(&exec_cmd(&t, &opts, PREFLIGHT_SH)).stdout).unwrap();
    assert_eq!(pf.arch, "x86_64");
    assert!(!pf.installed);
    println!("ok    preflight: {:?}", problems(&pf, ServerMode::LinuxNative).iter().map(|p| p.id).collect::<Vec<_>>());

    // install (upload, .env, native.sh install, start)
    let tgz = dir.join("runtime.tar.gz");
    let digest = runtime_tarball(&rt, &tgz, &SystemRunner).unwrap();
    run_steps(server_job_plan(&s, &opts, "server_install", &tgz, &digest).unwrap().1);
    println!("ok    install");

    // tunnel + status API through it
    let targets = Targets::new(dir.clone());
    targets.sync(&cfg.servers);
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut healthy = false;
    while Instant::now() < deadline {
        if let (Link::Connected { .. }, Some(sec)) = (targets.link(&s.id), targets.secrets(&s.id)) {
            assert_eq!(sec.version.as_deref(), Some(digest.as_str()));
            assert!(sec.token.is_some() && sec.password.is_some());
            if api_with(&s, &sec).health() {
                healthy = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(healthy, "status API through the tunnel: {:?}", targets.link(&s.id));
    println!("ok    tunnel 127.0.0.1:{} → status API healthy", s.port_base + 2);
    // the tunnel must stay up (a ControlMaster used to take it over: Final review C1)
    let since = match targets.link(&s.id) { Link::Connected { since } => since, other => panic!("{other:?}") };
    std::thread::sleep(Duration::from_secs(12));
    assert_eq!(targets.link(&s.id), Link::Connected { since }, "tunnel dropped");
    assert!(api_with(&s, &targets.secrets(&s.id).unwrap()).health());
    println!("ok    tunnel still up after 12 s");

    // update: a new digest lands in VERSION, the token stays
    let before = read_secrets(&dir, &s).unwrap();
    run_steps(server_job_plan(&s, &opts, "server_update", &tgz, "e2e-new-digest").unwrap().1);
    let after = read_secrets(&dir, &s).unwrap();
    assert_eq!(after.version.as_deref(), Some("e2e-new-digest"));
    assert_eq!(after.token, before.token, "update keeps the server's .env");
    println!("ok    update");

    // a changed host key is refused and classified
    targets.stop(&s.id);
    let kh = opts.known_hosts();
    let good = std::fs::read_to_string(&kh).unwrap();
    let other = dir.join("other_key");
    assert!(SystemRunner.run(&smt5_core::Cmd::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(other.display().to_string())).ok());
    let other_pub = std::fs::read_to_string(other.with_extension("pub")).unwrap();
    let host_field = good.split_whitespace().next().unwrap();
    let fake_line = format!("{host_field} {}\n", other_pub.split_whitespace().take(2).collect::<Vec<_>>().join(" "));
    std::fs::write(&kh, fake_line).unwrap();
    let _ = SystemRunner.run(&smt5_core::remote::control_exit_cmd(&t, &opts));
    let o = SystemRunner.run(&exec_cmd(&t, &opts, "true"));
    assert_eq!(classify_ssh_error(&o.stderr), SshError::HostKeyChanged, "{}", o.stderr);
    std::fs::write(&kh, good).unwrap();
    println!("ok    host key change refused");

    // uninstall removes everything
    run_steps(server_job_plan(&s, &opts, "server_uninstall", &tgz, "").unwrap().1);
    assert!(!SystemRunner.run(&exec_cmd(&t, &opts, "test -e \"$HOME/siliconmetatrader5\"")).ok());
    println!("ok    uninstall");
    targets.stop_all();
    let _ = std::fs::remove_dir_all(dir);
}
