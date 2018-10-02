use env_logger;
use libc;
use std::fmt;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::prelude::v1::*;
use std::process::Command;

use driverkit::DriverControl;
use driverkit::MsrInterface;

use super::{PTInfo, ProcessorTraceController};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

struct LinuxMsrInterface {
    cpu: usize,
    msr_file: File,
}

impl LinuxMsrInterface {
    fn new(cpuid: usize) -> LinuxMsrInterface {
        // /dev/cpu/CPUNUM/msr provides an interface to read and write the
        // model-specific registers (MSRs) of an x86 CPU.  CPUNUM is the number
        // of the CPU to access as listed in /proc/cpuinfo.
        let load_module = Command::new("modprobe")
            .args(&["msr"])
            .output()
            .expect("failed to execute process");
        assert!(load_module.status.success());

        let msr_path = format!("/dev/cpu/{}/msr", cpuid);
        let msr_file = OpenOptions::new()
            .write(true)
            .read(true)
            .open(msr_path)
            .expect("Can't open file");
        LinuxMsrInterface {
            cpu: cpuid,
            msr_file: msr_file,
        }
    }
}

impl fmt::Debug for LinuxMsrInterface {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "LinuxMsrInterface @ core {}", self.cpu)
    }
}

impl MsrInterface for LinuxMsrInterface {
    // The register access is done by opening the file and seeking to the
    // MSR number as offset in the file, and then reading or writing in
    // chunks of 8 bytes.  An I/O transfer of more than 8 bytes means
    // multiple reads or writes of the same register.

    unsafe fn write(&mut self, msr: u32, value: u64) {
        let pos = self
            .msr_file
            .seek(SeekFrom::Start(msr as u64))
            .expect("Can't seek");
        assert!(pos == msr.into());

        let mut contents: Vec<u8> = vec![];
        contents
            .write_u64::<LittleEndian>(value)
            .expect("Can't serialize MSR value");
        assert_eq!(contents.len(), 8, "Write exactly 8 bytes");
        self.msr_file
            .write(&contents)
            .expect(format!("Can't write MSR 0x{:x} with 0x{:x}", msr, value).as_str());

        debug!("wrmsr(0x{:x}, 0x{:x})", msr, value);
    }

    unsafe fn read(&mut self, msr: u32) -> u64 {
        let pos = self
            .msr_file
            .seek(SeekFrom::Start(msr as u64))
            .expect("Can't seek");
        assert!(pos == msr.into());
        let mut raw_value: Vec<u8> = vec![0; 8];
        self.msr_file
            .read(&mut raw_value)
            .expect("Can't read MSR value");
        let value = raw_value
            .as_slice()
            .read_u64::<LittleEndian>()
            .expect("Can't parse msr value");
        debug!("rdmsr(0x{:x}) -> 0x{:x}", msr, value);
        value
    }
}

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
fn trace_start() {
    let cpu: usize = 2;
    pin_thread(cpu);
    let mut linux_msr: LinuxMsrInterface = LinuxMsrInterface::new(cpu);
    let ptinfo = PTInfo::new().unwrap();

    debug!("{:?}", ptinfo);

    let mut controller = ProcessorTraceController::new(&mut linux_msr);
    controller.init();
    controller.attach();

    controller.start();
    for i in 0..200 {
        foo(i);
    }
    controller.stop();
    assert!(controller.current_offset() > 0x1000);

    controller.detach();

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
