//! Treating subnormal floats as zero on the current thread.
//!
//! A recursive filter fed exact zeros decays into the subnormal range and can
//! stay there, where some CPUs (Intel among them) do arithmetic on each value
//! through a slow path, many times the cost of the normal one.

/// Make the current thread's floating-point unit flush subnormal results to
/// zero, and on x86-64 treat subnormal inputs as zero too.
///
/// The setting belongs to the thread, so it is applied on the thread that
/// does the arithmetic. It is cheap enough to apply at the start of every
/// buffer.
pub fn flush_subnormals_to_zero() {
    #[cfg(target_arch = "x86_64")]
    {
        /// MXCSR: flush-to-zero (bit 15) and denormals-are-zero (bit 6).
        const FTZ_DAZ: u32 = (1 << 15) | (1 << 6);
        let mut csr: u32 = 0;
        // SAFETY: reads and writes only the MXCSR register's rounding and
        // flush controls through a stack local; changes no memory and no
        // other register.
        unsafe {
            std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack, preserves_flags));
            csr |= FTZ_DAZ;
            std::arch::asm!("ldmxcsr [{}]", in(reg) &csr, options(nostack, preserves_flags, readonly));
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        /// FPCR: flush-to-zero (bit 24).
        const FZ: u64 = 1 << 24;
        let mut fpcr: u64;
        // SAFETY: reads and writes only the FPCR register's flush control;
        // changes no memory.
        unsafe {
            std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags));
            fpcr |= FZ;
            std::arch::asm!("msr fpcr, {}", in(reg) fpcr, options(nomem, nostack, preserves_flags));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    /// Once flushing is on, a result that would be subnormal is zero, and a
    /// normal one is untouched.
    #[test]
    fn subnormal_results_become_zero() {
        flush_subnormals_to_zero();
        let smallest_normal = black_box(f32::MIN_POSITIVE);
        assert_eq!(black_box(smallest_normal * 0.5), 0.0);
        assert_eq!(black_box(smallest_normal * 2.0), 2.0 * f32::MIN_POSITIVE);
    }
}
