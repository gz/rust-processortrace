use env_logger;
use libc;
use std::prelude::v1::*;
use std::process::Command;

use super::{PTInfo, ProcessorTraceController, TraceDump, TraceDumpControl};

use driverkit::{DriverControl, MsrWriter};

#[cfg(target_os = "linux")]
fn pin_thread(core_id: usize) {
    use nix::sched::{sched_setaffinity, CpuSet};
    use nix::unistd::Pid;

    let mut affinity_set = CpuSet::new();
    affinity_set.set(core_id).expect("Can't set PU in core set");

    sched_setaffinity(Pid::from_raw(0i32), &affinity_set).expect("Can't pin app thread to core");
}

#[cfg(not(target_os = "linux"))]
fn pin_thread(_core_id: usize) {
    error!("Pinning threads not supported!");
}

#[test]
fn ptinfo() {
    let _ = env_logger::init();
    let ptinfo = PTInfo::new().unwrap();
    debug!("{:?}", ptinfo);
}

#[inline(never)]
fn foo(i: usize) {
    if i > 200 {
        println!("i = {}", i);
    }
}

#[test]
fn trace_fn() {
    let cpu: usize = 3;
    pin_thread(cpu);
    let mut msr_iface: MsrWriter = MsrWriter::new(cpu);
    let mut controller = ProcessorTraceController::new(&mut msr_iface);
    controller.init();
    controller.attach();

    let dump: TraceDump = controller.trace(|| {
        let mut a = 0;
        for i in 0..200 {
            foo(i);
        }
        println!("a = {}", a);
    });

    dump.save("trace_fn");

    controller.destroy();
}

#[test]
fn trace_raw() {
    let cpu: usize = 2;
    pin_thread(cpu);
    let mut linux_msr: MsrWriter = MsrWriter::new(cpu);

    let mut controller = ProcessorTraceController::new(&mut linux_msr);
    controller.init();
    controller.attach();

    controller.start();
    for i in 0..200 {
        foo(i);
    }
    controller.stop();
    assert!(controller.current_offset() > 0x1000);

    use std::fs::File;
    use std::io::prelude::*;

    let mut file = File::create("trace.dump").unwrap();
    file.write_all(controller.buffer.as_slice() as &[u8])
        .unwrap();

    let pid = unsafe { libc::getpid() };
    let _c = Command::new("cp")
        .arg(format!("/proc/{}/maps", pid))
        .arg("trace.map")
        .output()
        .expect("failed to execute process");

    controller.destroy();
}
