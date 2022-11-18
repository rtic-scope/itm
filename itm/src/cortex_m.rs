#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The possible local timestamp options.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LocalTimestampOptions {
    /// Disable local timestamps.
    Disabled,
    /// Enable local timestamps and use no prescaling.
    Enabled,
    /// Enable local timestamps and set the prescaler to divide the
    /// reference clock by 4.
    EnabledDiv4,
    /// Enable local timestamps and set the prescaler to divide the
    /// reference clock by 16.
    EnabledDiv16,
    /// Enable local timestamps and set the prescaler to divide the
    /// reference clock by 64.
    EnabledDiv64,
}

/// Active exception number
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum VectActive {
    /// Thread mode
    ThreadMode,

    /// Processor core exception (internal interrupts)
    Exception(Exception),

    /// Device specific exception (external interrupts)
    Interrupt {
        /// Interrupt number. This number is always within half open range `[0, 512)` (9 bit)
        irqn: u16,
    },
}

impl VectActive {
    /// Converts a vector number into `VectActive`
    #[inline]
    pub fn from(vect_active: u16) -> Option<Self> {
        Some(match vect_active {
            0 => VectActive::ThreadMode,
            2 => VectActive::Exception(Exception::NonMaskableInt),
            3 => VectActive::Exception(Exception::HardFault),
            4 => VectActive::Exception(Exception::MemoryManagement),
            5 => VectActive::Exception(Exception::BusFault),
            6 => VectActive::Exception(Exception::UsageFault),
            7 => VectActive::Exception(Exception::SecureFault),
            11 => VectActive::Exception(Exception::SVCall),
            12 => VectActive::Exception(Exception::DebugMonitor),
            14 => VectActive::Exception(Exception::PendSV),
            15 => VectActive::Exception(Exception::SysTick),
            irqn if (16..512).contains(&irqn) => VectActive::Interrupt { irqn: irqn - 16 },
            _ => return None,
        })
    }
}

/// Processor core exceptions (internal interrupts)
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Exception {
    /// Non maskable interrupt
    NonMaskableInt,

    /// Hard fault interrupt
    HardFault,

    /// Memory management interrupt (not present on Cortex-M0 variants)
    MemoryManagement,

    /// Bus fault interrupt (not present on Cortex-M0 variants)
    BusFault,

    /// Usage fault interrupt (not present on Cortex-M0 variants)
    UsageFault,

    /// Secure fault interrupt (only on ARMv8-M)
    SecureFault,

    /// SV call interrupt
    SVCall,

    /// Debug monitor interrupt (not present on Cortex-M0 variants)
    DebugMonitor,

    /// Pend SV interrupt
    PendSV,

    /// System Tick interrupt
    SysTick,
}

impl Exception {
    /// Returns the IRQ number of this `Exception`
    ///
    /// The return value is always within the closed range `[-1, -14]`
    #[inline]
    pub fn irqn(self) -> i8 {
        match self {
            Exception::NonMaskableInt => -14,
            Exception::HardFault => -13,
            Exception::MemoryManagement => -12,
            Exception::BusFault => -11,
            Exception::UsageFault => -10,
            Exception::SecureFault => -9,
            Exception::SVCall => -5,
            Exception::DebugMonitor => -4,
            Exception::PendSV => -2,
            Exception::SysTick => -1,
        }
    }
}
