#![feature(test)]
#![no_std]
extern crate driverkit;
extern crate rlibc;
extern crate x86;

#[macro_use]
extern crate log;

#[cfg(test)]
#[macro_use]
extern crate std;
#[cfg(test)]
extern crate byteorder;
#[cfg(test)]
extern crate env_logger;
#[cfg(test)]
extern crate libc;
#[cfg(test)]
extern crate nix;

#[cfg(test)]
extern crate test;

#[cfg(test)]
mod tests;

use driverkit::mem::DevMem;
use driverkit::{DriverControl, MsrInterface};

use x86::msr::{
    MSR_IA32_ADDR0_END, MSR_IA32_ADDR0_START, MSR_IA32_ADDR1_END, MSR_IA32_ADDR1_START,
    MSR_IA32_ADDR2_END, MSR_IA32_ADDR2_START, MSR_IA32_ADDR3_END, MSR_IA32_ADDR3_START,
    MSR_IA32_RTIT_CTL, MSR_IA32_RTIT_OUTPUT_BASE, MSR_IA32_RTIT_OUTPUT_MASK_PTRS,
    MSR_IA32_RTIT_STATUS,
};

macro_rules! bit {
    ($x:expr) => {
        1 << $x
    };
}

// Bits of MSR_IA32_RTIT_CTL
const TRACE_EN: u64 = bit!(0);
const CYC_EN: u64 = bit!(1);
const CTL_OS: u64 = bit!(2);
const CTL_USER: u64 = bit!(3);
const PWR_EVT_EN: u64 = bit!(4);
const FUP_ON_PTW: u64 = bit!(5);
const FABRIC_EN: u64 = bit!(6);
const CR3_FILTER: u64 = bit!(7);
const TOPA: u64 = bit!(8);
const MTC_EN: u64 = bit!(9);
const TSC_EN: u64 = bit!(10);
const DIS_RETC: u64 = bit!(11);
const PTW_EN: u64 = bit!(12);
const BRANCH_EN: u64 = bit!(13);

const MTC_SHIFT: u64 = 14;
const CYC_SHIFT: u64 = 19;
const PSB_SHIFT: u64 = 24;

const ADDR0_SHIFT: u64 = 32u64;
const ADDR1_SHIFT: u64 = 36u64;
const ADDR2_SHIFT: u64 = 40u64;
const ADDR3_SHIFT: u64 = 44u64;

const ADDR0_MASK: u64 = 0xf << ADDR0_SHIFT;
const ADDR1_MASK: u64 = 0xf << ADDR1_SHIFT;
const ADDR2_MASK: u64 = 0xf << ADDR2_SHIFT;
const ADDR3_MASK: u64 = 0xf << ADDR3_SHIFT;

/// MSR_IA32_RTIT_STATUS Error bit
const PT_ERROR: u64 = bit!(4);

#[derive(Debug, Default)]
struct PTInfo {
    has_topa: bool,
    has_cr3_match: bool,
    mtc_freq_mask: u16,
    cyc_thresh_mask: u16,
    psb_freq_mask: u16,
    addr_range_num: u8,
    addr_cfg_max: usize,
}

impl PTInfo {
    fn new() -> Option<PTInfo> {
        let cpuid = x86::cpuid::CpuId::new();
        let ptinfo = cpuid.get_processor_trace_info()?;

        Some(PTInfo {
            has_topa: ptinfo.has_topa(),
            has_cr3_match: ptinfo.has_rtit_cr3_match(),
            mtc_freq_mask: ptinfo.supported_mtc_period_encodings(),
            cyc_thresh_mask: ptinfo.supported_cycle_threshold_value_encodings(),
            psb_freq_mask: ptinfo.supported_psb_frequency_encodings(),
            addr_range_num: ptinfo.configurable_address_ranges(),
            addr_cfg_max: if ptinfo.has_ip_tracestop_filtering() {
                2
            } else {
                0
            },
        })
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum FilterConfig {
    Off,
    // Filter for this range (start, end)
    Trace(u64, u64),
    // Stop for this range (start, end)
    TraceStop(u64, u64),
}

pub struct ProcessorTraceController<'a> {
    running: bool,
    current_offset: u32,
    buffer: DevMem,
    msr_interface: &'a mut MsrInterface,

    /// Don't enable branch tracing (if supported)
    pub disable_branch: bool,

    /// Set to false to avoid tracing user space
    pub user: bool,

    /// Set to false to avoid tracing kernel space
    pub kernel: bool,

    /// Set to false to not use TSC
    pub tsc_en: bool,

    /// Set to false to disable return compression
    pub dis_retc: bool,

    /// Clear PT buffer before start
    pub clear_on_start: bool,

    /// Send cycle packets at every 2^(n-1) cycles (if supported)
    pub cyc_thresh: u64,

    /// Enable MTC packets at frequency 2^(n-1) (if supported)
    pub mtc_freq: u64,

    /// Send PSB packets every 2K^n bytes (if supported)
    pub psb_freq: u64,

    /// Mode of address range 0 filter
    pub addr0_cfg: FilterConfig,

    /// Mode of address range 1 filter
    pub addr1_cfg: FilterConfig,

    /// Mode of address range 2 filter
    pub addr2_cfg: FilterConfig,

    /// Mode of address range 3 filter
    pub addr3_cfg: FilterConfig,
}

impl<'a> DriverControl for ProcessorTraceController<'a> {
    fn attach(&mut self) {
        unsafe {
            let ctl = self.msr_interface.read(MSR_IA32_RTIT_CTL);
            if ctl & TRACE_EN > 0 {
                warn!("Processor tracing is already enabled, we're forcefully taking over");
            }
        }

        self.install_trace_buffer();
        unsafe {
            self.msr_interface.write(MSR_IA32_RTIT_STATUS, 0);
        }
    }

    fn set_sleep_level(&mut self, _level: usize) {}

    fn detach(&mut self) {
        self.stop();
        unsafe {
            self.msr_interface.write(MSR_IA32_RTIT_OUTPUT_MASK_PTRS, 0);
        }
    }
}

impl<'a> ProcessorTraceController<'a> {
    pub fn new(msr_interface: &'a mut MsrInterface) -> ProcessorTraceController<'a> {
        ProcessorTraceController {
            running: false,
            current_offset: 0,
            buffer: DevMem::alloc(1024 * 1024 * 2).unwrap(),
            msr_interface: msr_interface,

            disable_branch: false,
            user: true,
            kernel: true,
            tsc_en: true,
            dis_retc: true,
            clear_on_start: true,
            cyc_thresh: 0,
            mtc_freq: 0,
            psb_freq: 0,
            addr0_cfg: FilterConfig::Off,
            addr1_cfg: FilterConfig::Off,
            addr2_cfg: FilterConfig::Off,
            addr3_cfg: FilterConfig::Off,
        }
    }

    pub fn start(&mut self) {
        info!("Starting PT");
        unsafe {
            let ptinfo = PTInfo::new().unwrap();

            let mut rtit_ctl = self.msr_interface.read(MSR_IA32_RTIT_CTL);
            if rtit_ctl & TRACE_EN > 0 {
                self.msr_interface
                    .write(MSR_IA32_RTIT_CTL, rtit_ctl & !TRACE_EN);
            }

            // Clear on start and trace was disabled
            if self.clear_on_start && !(rtit_ctl & TRACE_EN > 0) {
                rlibc::memset(self.buffer.as_mut_ptr(), 0, self.buffer.len());
                self.install_buffer_mask();
                self.msr_interface.write(MSR_IA32_RTIT_STATUS, 0);
            }

            rtit_ctl &= !(TRACE_EN
                | CYC_EN
                | CTL_OS
                | CTL_USER
                | PWR_EVT_EN
                | FUP_ON_PTW
                | FABRIC_EN
                | CR3_FILTER
                | TOPA
                | MTC_EN
                | TSC_EN
                | DIS_RETC
                | PTW_EN
                | BRANCH_EN
                | ADDR0_MASK
                | ADDR1_MASK
                | ADDR2_MASK
                | ADDR3_MASK);

            // Start tracing
            rtit_ctl |= TRACE_EN;

            if !self.disable_branch {
                rtit_ctl |= BRANCH_EN;
            }

            if self.tsc_en {
                rtit_ctl |= TSC_EN;
            }

            if self.kernel {
                rtit_ctl |= CTL_OS;
            }

            if self.user {
                rtit_ctl |= CTL_USER;
            }

            if self.dis_retc {
                rtit_ctl |= DIS_RETC;
            }

            if self.mtc_freq > 0 && ((1 << (self.mtc_freq - 1)) & ptinfo.mtc_freq_mask) > 0 {
                rtit_ctl |= ((self.mtc_freq - 1) << MTC_SHIFT) | MTC_EN;
            }

            if self.cyc_thresh > 0 && ((1 << (self.cyc_thresh - 1)) & ptinfo.cyc_thresh_mask) > 0 {
                rtit_ctl |= ((self.cyc_thresh - 1) << CYC_SHIFT) | CYC_EN;
            }

            if self.psb_freq > 0 && ((1 << (self.psb_freq - 1)) & ptinfo.psb_freq_mask) > 0 {
                rtit_ctl |= (self.psb_freq - 1) << PSB_SHIFT;
            }

            for &(i, cfg, shift, addr_start_msr, addr_end_msr) in [
                (
                    0u8,
                    self.addr0_cfg,
                    ADDR0_SHIFT,
                    MSR_IA32_ADDR0_START,
                    MSR_IA32_ADDR0_END,
                ),
                (
                    1u8,
                    self.addr1_cfg,
                    ADDR1_SHIFT,
                    MSR_IA32_ADDR1_START,
                    MSR_IA32_ADDR1_END,
                ),
                (
                    2u8,
                    self.addr2_cfg,
                    ADDR2_SHIFT,
                    MSR_IA32_ADDR2_START,
                    MSR_IA32_ADDR2_END,
                ),
                (
                    3u8,
                    self.addr3_cfg,
                    ADDR3_SHIFT,
                    MSR_IA32_ADDR3_START,
                    MSR_IA32_ADDR3_END,
                ),
            ]
                .iter()
            {
                if ptinfo.addr_range_num > i {
                    match cfg {
                        FilterConfig::Off => (),
                        FilterConfig::TraceStop(start, end) => {
                            rtit_ctl |= (1 << shift) as u64;
                            self.msr_interface.write(addr_start_msr, start);
                            self.msr_interface.write(addr_end_msr, end);
                        }
                        FilterConfig::Trace(start, end) => {
                            rtit_ctl |= (2 << shift) as u64;
                            self.msr_interface.write(addr_start_msr, start);
                            self.msr_interface.write(addr_end_msr, end);
                        }
                    }
                } else if cfg != FilterConfig::Off {
                    warn!(
                        "Ignore configuring address range filter {}, not supported.",
                        i
                    );
                }
            }

            debug!("Setting RTIT_CTL to 0x{:x}", rtit_ctl);
            self.msr_interface.write(MSR_IA32_RTIT_CTL, rtit_ctl);
            self.running = true;
        }
        info!("Started PT");
    }

    pub fn stop(&mut self) {
        if !self.running {
            return;
        }

        info!("Stopping ProcessorTraceController");
        unsafe {
            let ctl = self.msr_interface.read(MSR_IA32_RTIT_CTL);
            if (ctl & TRACE_EN) == 0 {
                debug!(
                    "Trace was not enabled on stop(), MSR_IA32_RTIT_CTL = {}",
                    ctl
                );
            }

            let status = self.msr_interface.read(MSR_IA32_RTIT_STATUS);
            if status & PT_ERROR > 0 {
                error!(
                    "ProcessorTraceController reports error, MSR_IA32_RTIT_STATUS = {}",
                    status
                );
            }

            self.msr_interface.write(MSR_IA32_RTIT_CTL, 0);
            self.msr_interface.write(MSR_IA32_RTIT_STATUS, 0);

            let offset = self.msr_interface.read(MSR_IA32_RTIT_OUTPUT_MASK_PTRS);
            self.current_offset = (offset >> 32) as u32;
            debug!(
                "Trace data gathered (at least) 0x{:x} bytes",
                self.current_offset
            );

            self.running = false;
        }
        info!("Stopped ProcessorTraceController");
    }

    pub fn current_offset(&self) -> u32 {
        self.current_offset
    }

    fn install_trace_buffer(&mut self) {
        debug!(
            "Set ProcessorTraceController install trace buffer virtual 0x{:x}, physical 0x{:x}",
            self.buffer.virtual_address(),
            self.buffer.physical_address()
        );

        unsafe {
            self.msr_interface
                .write(MSR_IA32_RTIT_OUTPUT_BASE, self.buffer.physical_address());
        }
        self.install_buffer_mask();
    }

    fn install_buffer_mask(&mut self) {
        assert!(self.buffer.len().is_power_of_two());

        unsafe {
            self.msr_interface.write(
                MSR_IA32_RTIT_OUTPUT_MASK_PTRS,
                (self.buffer.len() - 1) as u64,
            )
        }
    }
}
