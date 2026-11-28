//! Host-selected resource limits and scoped allocation accounting.
use crate::{
    diagnostic::{Category, Diagnostic, Span},
    Error,
};
use std::{
    cell::{Cell, RefCell},
    mem::size_of,
    rc::Rc,
};

pub(crate) const RANK_CEILING: usize = 16;
const DEPTH_CEILING: u16 = 128;
const TRACKED_MEMORY_LIMIT: &str = "tracked runtime memory limit exceeded";

/// Independent resource ceilings for one execution. A configuration is
/// validated before evaluation starts. Memory covers instrumented buffers,
/// not the process's total resident memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourcePolicy {
    pub max_rank: usize,
    pub max_elements: usize,
    pub max_memory: usize,
    pub max_work: usize,
    pub max_exact_bits: u64,
    pub max_numeric_iterations: usize,
    pub max_depth: u16,
}

impl ResourcePolicy {
    const DEFAULT: Self = Self {
        max_rank: RANK_CEILING,
        max_elements: 100_000,
        max_memory: 256 * 1024 * 1024,
        max_work: 1_000_000,
        max_exact_bits: 1_048_576,
        max_numeric_iterations: 100_000,
        max_depth: DEPTH_CEILING,
    };

    /// Validate the host configuration independently of the program.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |message: &str| {
            Diagnostic::new(Category::RuntimeDomain, Span::default(), message).into_error()
        };
        if !(1..=RANK_CEILING).contains(&self.max_rank) {
            return Err(invalid("runtime max_rank must be from 1 through 16"));
        }
        if !(1..=DEPTH_CEILING).contains(&self.max_depth) {
            return Err(invalid("runtime max_depth must be from 1 through 128"));
        }
        let portable = isize::MAX as usize;
        let max_elements = portable / size_of::<super::V>();
        if !(1..=max_elements).contains(&self.max_elements) {
            return Err(invalid(
                "runtime max_elements must be positive and fit a host-sized buffer",
            ));
        }
        if !(1..=portable).contains(&self.max_memory) {
            return Err(invalid(
                "runtime max_memory must be positive and fit in isize",
            ));
        }
        if self.max_work == 0 {
            return Err(invalid("runtime max_work must be positive"));
        }
        if self.max_exact_bits == 0 || self.max_exact_bits as u128 > (portable as u128) * 8 {
            return Err(invalid(
                "runtime max_exact_bits must be positive and fit a host-sized buffer",
            ));
        }
        if self.max_numeric_iterations == 0 {
            return Err(invalid("runtime max_numeric_iterations must be positive"));
        }
        Ok(())
    }
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Host configuration. Disabling optimization selects the generic typed
/// executor, not the legacy interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeOptions {
    pub resources: ResourcePolicy,
    pub optimize: bool,
}

impl RuntimeOptions {
    const DEFAULT: Self = Self {
        resources: ResourcePolicy::DEFAULT,
        optimize: true,
    };

    pub fn validate(&self) -> Result<(), Error> {
        self.resources.validate()
    }
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

struct Ledger {
    used: Cell<usize>,
    cap: usize,
}

thread_local! {
    static OPTIONS: Cell<RuntimeOptions> = const { Cell::new(RuntimeOptions::DEFAULT) };
    static LEDGER: RefCell<Option<Rc<Ledger>>> = const { RefCell::new(None) };
}

pub(super) fn limits() -> ResourcePolicy {
    OPTIONS.get().resources
}

pub(super) fn optimized() -> bool {
    OPTIONS.get().optimize
}

/// Check a numerical algorithm's bounded iteration work, independently of
/// evaluator fuel and any options passed as Flux values.
pub(super) fn check_iterations(n: usize) -> super::NumericResult<()> {
    if n > limits().max_numeric_iterations {
        return Err("runtime numeric iteration limit exceeded");
    }
    Ok(())
}

pub(super) fn available_memory() -> usize {
    LEDGER.with_borrow(|ledger| {
        ledger
            .as_deref()
            .map_or_else(|| limits().max_memory, |state| state.cap - state.used.get())
    })
}

pub(super) struct Scope {
    previous_options: RuntimeOptions,
    previous_ledger: Option<Rc<Ledger>>,
}

pub(super) fn enter(options: RuntimeOptions) -> Result<Scope, Error> {
    options.validate()?;
    let ledger = Ledger {
        used: Cell::new(0),
        cap: options.resources.max_memory,
    };
    Ok(Scope {
        previous_options: OPTIONS.replace(options),
        previous_ledger: LEDGER.replace(Some(Rc::new(ledger))),
    })
}

impl Drop for Scope {
    fn drop(&mut self) {
        OPTIONS.set(self.previous_options);
        LEDGER.replace(self.previous_ledger.take());
    }
}

/// A lease held until a tracked buffer is released. Sharing an allocation
/// shares its one lease too. A nested run cannot release its caller's lease.
pub(super) struct MemoryReservation {
    reservation: Option<(Rc<Ledger>, usize)>,
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        if let Some((ledger, bytes)) = &self.reservation {
            ledger.used.set(ledger.used.get() - bytes);
        }
    }
}

pub(super) fn reserve(bytes: usize) -> super::NumericResult<MemoryReservation> {
    if bytes > available_memory() {
        return Err(TRACKED_MEMORY_LIMIT);
    }
    let reservation = LEDGER.with_borrow(|ledger| {
        ledger.as_ref().map(|ledger| {
            ledger.used.set(ledger.used.get() + bytes);
            (Rc::clone(ledger), bytes)
        })
    });
    Ok(MemoryReservation { reservation })
}

pub(super) fn reserve_for<T>(n: usize) -> super::NumericResult<MemoryReservation> {
    reserve(n.checked_mul(size_of::<T>()).ok_or(TRACKED_MEMORY_LIMIT)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_runs_restore_options_and_isolate_live_allocations() {
        let options = RuntimeOptions {
            resources: ResourcePolicy {
                max_memory: 8,
                ..Default::default()
            },
            optimize: false,
        };
        let _outer = enter(options).unwrap();
        assert!(!optimized());
        let reserved = reserve_for::<u64>(1).unwrap();
        assert_eq!(available_memory(), 0);
        assert!(reserve(1).is_err());
        {
            let _inner = enter(RuntimeOptions::default()).unwrap();
            assert!(optimized());
            let _temporary = reserve(16).unwrap();
            assert_eq!(available_memory(), limits().max_memory - 16);
        }
        assert!(!optimized());
        assert_eq!(available_memory(), 0);
        drop(reserved);
        assert_eq!(available_memory(), 8);
    }

    #[test]
    fn zero_and_unsupported_configuration_are_invalid() {
        for policy in [
            ResourcePolicy {
                max_rank: 0,
                ..Default::default()
            },
            ResourcePolicy {
                max_rank: RANK_CEILING + 1,
                ..Default::default()
            },
            ResourcePolicy {
                max_elements: 0,
                ..Default::default()
            },
            ResourcePolicy {
                max_memory: 0,
                ..Default::default()
            },
            ResourcePolicy {
                max_work: 0,
                ..Default::default()
            },
            ResourcePolicy {
                max_exact_bits: 0,
                ..Default::default()
            },
            ResourcePolicy {
                max_numeric_iterations: 0,
                ..Default::default()
            },
            ResourcePolicy {
                max_depth: DEPTH_CEILING + 1,
                ..Default::default()
            },
        ] {
            assert!(policy.validate().is_err());
        }
    }

    #[test]
    fn generated_and_wrapped_values_obey_configured_exact_caps() {
        for (bits, source) in [
            (4, r#"<<"dt"; date := dt.days "2026-11-28";"#),
            (
                4,
                r#"<<"arr"; id(x:z):z := x;
                order := arr.grade &id [1z 1z 1z 1z 1z 1z 1z 1z 1z 1z 1z 1z 1z 1z 1z 1z];"#,
            ),
            (
                2,
                r#"a := form [3n] [1z 2z 3z]; hold(x,y:z):z := x;
                scan := \: &hold 0z a;
                dims := @ &dim scan;"#,
            ),
        ] {
            for optimize in [false, true] {
                let options = RuntimeOptions {
                    resources: ResourcePolicy {
                        max_exact_bits: bits,
                        ..Default::default()
                    },
                    optimize,
                };
                let error = crate::run_source_with_options(source, options).unwrap_err();
                assert!(
                    error.message.contains("resource bit limit"),
                    "{source}: {error}"
                );
            }
        }
    }
}
