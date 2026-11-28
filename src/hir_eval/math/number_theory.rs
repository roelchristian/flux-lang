//! Exact integer operations. Resource exhaustion is distinct from a proof of
//! compositeness, and a factorization is returned only with proven prime factors.
use super::*;
use num_bigint::BigUint;
use num_traits::One;
use std::cell::Cell;

const THEORY_LIMIT: &str = "number-theory resource limit exceeded";
const MAX_WORK: u64 = 4_000_000;
const MAX_LIFT_BITS: u64 = 64 * 1024 * 1024 * 8;
const MAX_COMBINATORIC_STEPS: u64 = 100_000;
const MAX_PROOF_DEPTH: u8 = 24;
const SMALL_TRIAL: u32 = 257;
const CERTIFICATE_TRIAL: u32 = 8191;

// Sorenson and Webster, Math. Comp. 86 (2017), doi:10.1090/mcom/3134:
// this is the first strong pseudoprime to all twelve bases. A survivor
// strictly below it is prime; at or above it a certificate is required.
const FIRST_PSEUDOPRIME: u128 = 318_665_857_834_031_151_167_461;
const BASES: [u32; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];

fn natural(value: &V) -> NumericResult<BigUint> {
    wide05::integral(value)?
        .to_biguint()
        .ok_or("expected a natural number")
}

fn natural_value(value: BigUint) -> NumericResult<V> {
    wide05::ratio_value(BigRational::from_integer(value.into()), Domain::Natural)
}

fn bounded(value: BigUint, bits: u64) -> NumericResult<BigUint> {
    if value.bits() > bits {
        Err(THEORY_LIMIT)
    } else {
        Ok(value)
    }
}

fn multiply(a: &BigUint, b: &BigUint, bits: u64) -> NumericResult<BigUint> {
    if a.is_zero() || b.is_zero() {
        return Ok(BigUint::zero());
    }
    if a.bits() + b.bits() - 1 > bits {
        return Err(THEORY_LIMIT);
    }
    bounded(a * b, bits)
}

fn steps(n: &BigUint) -> NumericResult<u64> {
    let n = n
        .to_u64()
        .filter(|&n| n <= MAX_COMBINATORIC_STEPS)
        .ok_or(THEORY_LIMIT)?;
    resources::check_iterations(n as usize)?;
    Ok(n)
}

/// A balanced product avoids repeatedly multiplying a nearly maximal
/// integer by a single-word factor near the factorial size limit.
fn product(first: &BigUint, count: u64, bits: u64) -> NumericResult<BigUint> {
    match count {
        0 => Ok(BigUint::one()),
        1 => bounded(first.clone(), bits),
        n => {
            if first.bits().saturating_sub(1).saturating_mul(n) >= bits {
                return Err(THEORY_LIMIT);
            }
            let half = n / 2;
            let left = product(first, half, bits)?;
            let right = product(&(first + half), n - half, bits)?;
            multiply(&left, &right, bits)
        }
    }
}

fn charge_product(start: &BigUint, steps: u64, lift: Option<&LiftBudget>) -> NumericResult<()> {
    if let Some(lift) = lift {
        lift.work
            .charge(steps.saturating_mul(words(&(start + steps))))?;
    }
    Ok(())
}

fn choose(n: BigUint, k: BigUint, lift: Option<&LiftBudget>) -> NumericResult<BigUint> {
    if k > n {
        return Ok(BigUint::zero());
    }
    let complement = &n - &k;
    let k = steps(&k.min(complement))?;
    if k < 2 {
        return Ok(if k == 0 { BigUint::one() } else { n });
    }
    let first = &n - k + 1u8;
    charge_product(&BigUint::one(), k, lift)?;
    charge_product(&first, k, lift)?;
    let denominator = product(&BigUint::one(), k, wide05::max_bits().saturating_mul(2))?;
    // If the unreduced numerator exceeded this limit, dividing by the
    // already known denominator could not save the bounded final result.
    let numerator = product(
        &first,
        k,
        wide05::max_bits().saturating_add(denominator.bits()),
    )?;
    bounded(numerator / denominator, wide05::max_bits())
}

fn fibonacci(n: &BigUint, lift: Option<&LiftBudget>) -> NumericResult<BigUint> {
    // F(k+2) >= 2F(k) for k >= 1 bounds the largest useful index.
    let n = n
        .to_u64()
        .filter(|&n| n <= wide05::max_bits().saturating_mul(2).saturating_add(2))
        .ok_or(THEORY_LIMIT)?;
    if let Some(lift) = lift {
        lift.work.charge(n.div_ceil(64))?;
    }
    // F(n+1) may have one more bit than F(n).
    let bits = wide05::max_bits().saturating_add(1);
    let (mut a, mut b) = (BigUint::zero(), BigUint::one());
    for i in (0..(u64::BITS - n.leading_zeros())).rev() {
        let c = multiply(&a, &((&b << 1usize) - &a), bits)?;
        let d = bounded(multiply(&a, &a, bits)? + multiply(&b, &b, bits)?, bits)?;
        (a, b) = if (n >> i) & 1 == 0 {
            (c, d)
        } else {
            let next = bounded(&c + &d, bits)?;
            (d, next)
        };
    }
    bounded(a, wide05::max_bits())
}

struct Budget(Cell<u64>);
impl Budget {
    fn new() -> Self {
        Self(Cell::new(MAX_WORK))
    }

    fn charge(&self, n: u64) -> NumericResult<()> {
        self.0.set(self.0.get().checked_sub(n).ok_or(THEORY_LIMIT)?);
        Ok(())
    }
}

/// Lifted calls share one allowance rather than starting over at each cell.
pub(super) struct LiftBudget {
    work: Budget,
    retained: Cell<u64>,
}
impl LiftBudget {
    pub(super) fn new() -> Self {
        Self {
            work: Budget::new(),
            retained: Cell::new(0),
        }
    }
}

fn value_bits(v: &V) -> u64 {
    match v {
        V::Wide(w) => match w.as_ref() {
            Wide::Exact { ratio, .. } => ratio.numer().bits() + ratio.denom().bits(),
            Wide::Decimal(d) => d.storage_bits(),
        },
        _ => 0,
    }
}

fn words(n: &BigUint) -> u64 {
    n.bits().div_ceil(64).max(1)
}

fn modpow(
    base: &BigUint,
    exponent: &BigUint,
    modulus: &BigUint,
    budget: &Budget,
) -> NumericResult<BigUint> {
    if modulus.is_zero() {
        return Err("math.modpow modulus must be positive");
    }
    if modulus.is_one() {
        return Ok(BigUint::zero());
    }
    if exponent.is_zero() {
        return Ok(BigUint::one());
    }
    let width = words(modulus);
    budget.charge(words(base).saturating_mul(width))?;
    let base = base % modulus;
    if base.is_zero() || base.is_one() || exponent.is_one() {
        return Ok(base);
    }
    budget.charge(exponent.bits().saturating_mul(width * width))?;
    Ok(base.modpow(exponent, modulus))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimeStatus {
    Prime,
    Composite,
    Unknown,
}

fn trial_divisor(n: &BigUint, budget: &Budget) -> NumericResult<Option<u32>> {
    if n.is_even() {
        return Ok(Some(2));
    }
    for candidate in (3..=SMALL_TRIAL).step_by(2) {
        budget.charge(words(n))?;
        if n % candidate == BigUint::zero() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn passes_strong_test(
    n: &BigUint,
    n_minus_one: &BigUint,
    odd: &BigUint,
    twos: u64,
    base: u32,
    budget: &Budget,
) -> NumericResult<bool> {
    let width = words(n);
    let mut x = modpow(&BigUint::from(base), odd, n, budget)?;
    if x.is_one() || x == *n_minus_one {
        return Ok(true);
    }
    for _ in 1..twos {
        budget.charge(width * width)?;
        x = (&x * &x) % n;
        if x == *n_minus_one {
            return Ok(true);
        }
        if x.is_one() {
            return Ok(false);
        }
    }
    Ok(false)
}

type Factors = Vec<(BigUint, u64)>;

/// Remove factors in increasing order. Repeated prime powers are compressed
/// until the caller's materialization budget is checked.
fn divide_out_small(n: &mut BigUint, budget: &Budget, limit: u32) -> NumericResult<Factors> {
    let mut factors = Vec::new();
    let twos = n.trailing_zeros().unwrap_or(0);
    if twos != 0 {
        *n >>= twos;
        factors.push((BigUint::from(2u8), twos));
    }
    for candidate in (3..=limit).step_by(2) {
        if n.is_one() {
            break;
        }
        let factor = BigUint::from(candidate);
        let mut exponent = 0u64;
        loop {
            budget.charge(words(n))?;
            let (q, rem) = n.div_rem(&factor);
            if !rem.is_zero() {
                break;
            }
            *n = q;
            exponent += 1;
        }
        // Any divisor here is prime: a smaller prime divisor would have
        // been removed before this candidate was tried.
        if exponent != 0 {
            factors.push((factor, exponent));
        }
    }
    Ok(factors)
}

fn classify(n: &BigUint, budget: &Budget, depth: u8) -> NumericResult<PrimeStatus> {
    use PrimeStatus::*;
    if n.bits() < 2 {
        return Ok(Composite);
    }
    if let Some(d) = trial_divisor(n, budget)? {
        return Ok(if n == &BigUint::from(d) {
            Prime
        } else {
            Composite
        });
    }
    let last = n - 1u8;
    let twos = last.trailing_zeros().expect("positive even integer");
    let odd = &last >> twos;
    for base in BASES {
        if !passes_strong_test(n, &last, &odd, twos, base, budget)? {
            return Ok(Composite);
        }
    }
    if n.to_u128().is_some_and(|n| n < FIRST_PSEUDOPRIME) {
        return Ok(Prime);
    }

    // Pocklington: prove more than sqrt(n) of n-1's factorization,
    // then certify every distinct prime in that known factor.
    if depth >= MAX_PROOF_DEPTH {
        return Ok(Unknown);
    }
    let mut residual = last.clone();
    let mut known = divide_out_small(&mut residual, budget, CERTIFICATE_TRIAL)?;
    let mut part = &last / &residual;
    let mut pending = vec![residual];
    while &part * &part <= *n {
        let Some(factor) = pending.pop().filter(|x| !x.is_one()) else {
            return Ok(Unknown);
        };
        match classify(&factor, budget, depth + 1)? {
            Prime => {
                part *= &factor;
                known.push((factor, 1));
            }
            Composite => {
                let divisor = rho(&factor, budget)?;
                pending.push(&factor / &divisor);
                pending.push(divisor);
            }
            Unknown => return Ok(Unknown),
        }
    }
    known.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    known.dedup_by(|a, b| a.0 == b.0);
    for (q, _) in known {
        let exponent = &last / q;
        let mut witnessed = false;
        for a in BASES {
            let base = BigUint::from(a);
            if !modpow(&base, &last, n, budget)?.is_one() {
                return Ok(Composite);
            }
            let x = modpow(&base, &exponent, n, budget)?;
            if x.is_zero() {
                return Ok(Composite);
            }
            let d = (x - 1u8).gcd(n);
            if d.is_one() {
                witnessed = true;
                break;
            }
            if d != *n {
                return Ok(Composite);
            }
        }
        if !witnessed {
            return Ok(Unknown);
        }
    }
    Ok(Prime)
}

fn rho(n: &BigUint, budget: &Budget) -> NumericResult<BigUint> {
    // Fixed starts ensure deterministic work and errors. Floyd stepping
    // finds a split, not a certificate; both parts are checked afterwards.
    if let Some(m) = n.to_u64() {
        for c in 1..=64u64 {
            let step =
                |x: u64| ((u128::from(x) * u128::from(x) + u128::from(c)) % u128::from(m)) as u64;
            let (mut x, mut y) = (2, 2);
            loop {
                budget.charge(1)?;
                x = step(x);
                y = step(step(y));
                let d = x.abs_diff(y).gcd(&m);
                if d == m {
                    break;
                }
                if d > 1 {
                    return Ok(BigUint::from(d));
                }
            }
        }
    } else {
        let width = words(n);
        for c in 1..=64u32 {
            let step = |x: &BigUint| (x * x + c) % n;
            let mut x = BigUint::from(2u8);
            let mut y = x.clone();
            loop {
                budget.charge(4 * width * width)?;
                x = step(&x);
                y = step(&step(&y));
                let d = if x >= y { &x - &y } else { &y - &x }.gcd(n);
                if d == *n {
                    break;
                }
                if !d.is_one() {
                    return Ok(d);
                }
            }
        }
    }
    Err(THEORY_LIMIT)
}

fn factor(mut n: BigUint) -> NumericResult<Factors> {
    if n.is_zero() {
        return Err("zero has no finite prime factorization or divisor list");
    }
    let budget = Budget::new();
    let mut found = divide_out_small(&mut n, &budget, SMALL_TRIAL)?;
    let mut pending = vec![n];
    while let Some(n) = pending.pop() {
        if n.is_one() {
            continue;
        }
        if classify(&n, &budget, 0)? == PrimeStatus::Prime {
            found.push((n, 1));
        } else {
            let root = n.sqrt();
            let d = if &root * &root == n {
                root
            } else {
                rho(&n, &budget)?
            };
            pending.push(&n / &d);
            pending.push(d);
        }
    }
    found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut factors: Factors = Vec::new();
    for (p, count) in found {
        if let Some((_, previous)) = factors.last_mut().filter(|(last, _)| *last == p) {
            *previous += count;
        } else {
            factors.push((p, count));
        }
    }
    Ok(factors)
}

fn factor_values(factors: Factors) -> NumericResult<V> {
    let total: u64 = factors.iter().map(|(_, exponent)| exponent).sum();
    if total > COLLECTION_LIMIT as u64 {
        return Err("prime factor list exceeds the collection limit");
    }
    let mut out = Vec::with_capacity(total as usize);
    for (p, count) in factors {
        let value = natural_value(p)?;
        out.extend(std::iter::repeat_n(value, count as usize));
    }
    Ok(V::Sequence(out.into()))
}

fn divisor_values(n: BigUint, factors: Factors) -> NumericResult<V> {
    let total = factors.iter().try_fold(1u64, |size, (_, count)| {
        size.checked_mul(count + 1)
            .filter(|n| *n <= COLLECTION_LIMIT as u64)
            .ok_or("divisor list exceeds the collection limit")
    })?;
    if n.bits() * total > 64 * 1024 * 1024 * 8 {
        return Err("wide divisor list exceeds the resource limit (64 MiB of integer data)");
    }
    let mut values = Vec::with_capacity(total as usize);
    values.push(BigUint::one());
    for (p, count) in factors {
        let width = values.len();
        let mut power = BigUint::one();
        for _ in 0..count {
            power *= &p;
            for i in 0..width {
                values.push(&values[i] * &power);
            }
        }
    }
    values.sort_unstable();
    let values = values
        .into_iter()
        .map(natural_value)
        .collect::<NumericResult<Rc<_>>>()?;
    Ok(V::Sequence(values))
}

pub(super) fn apply(op: CoreOp, args: &[V], _ty: &Type) -> NumericResult<V> {
    calculate(op, args, &Budget::new(), None)
}

pub(super) fn apply_budgeted(
    op: CoreOp,
    args: &[V],
    _ty: &Type,
    budget: &LiftBudget,
) -> NumericResult<V> {
    let words = |v| value_bits(v).div_ceil(64).max(1);
    for input in args {
        budget.work.charge(words(input))?;
    }
    // Large GCD/LCM and square roots also need shared arithmetic, not
    // just acquisition of their (potentially shared) input values.
    match (op, args) {
        (CoreOp::MathGcd | CoreOp::MathLcm, [a, b]) => budget.work.charge(words(a) * words(b))?,
        (CoreOp::MathIsqrt, [a]) => budget.work.charge(words(a) * words(a))?,
        _ => {}
    }
    let output = calculate(op, args, &budget.work, Some(budget))?;
    let retained = budget.retained.get() + value_bits(&output);
    if retained > MAX_LIFT_BITS {
        return Err("number-theory lift exceeds the resource limit (64 MiB of integer data)");
    }
    budget.retained.set(retained);
    Ok(output)
}

fn calculate(op: CoreOp, args: &[V], work: &Budget, lift: Option<&LiftBudget>) -> NumericResult<V> {
    use CoreOp::*;
    let result = match (op, args) {
        (MathGcd | MathLcm, [a, b]) => {
            let (_, a) = wide05::integral(a)?.into_parts();
            let (_, b) = wide05::integral(b)?.into_parts();
            let g = a.gcd(&b);
            if op == MathGcd || a.is_zero() || b.is_zero() {
                if op == MathGcd {
                    g
                } else {
                    BigUint::zero()
                }
            } else {
                multiply(&(a / g), &b, wide05::max_bits())?
            }
        }
        (MathFact, [x]) => {
            let first = BigUint::one();
            let count = steps(&natural(x)?)?;
            charge_product(&first, count, lift)?;
            product(&first, count, wide05::max_bits())?
        }
        (MathChoose, [n, k]) => choose(natural(n)?, natural(k)?, lift)?,
        (MathPerm, [n, k]) => {
            let (n, k) = (natural(n)?, natural(k)?);
            if k > n {
                BigUint::zero()
            } else {
                let count = steps(&k)?;
                let first = n - k + 1u8;
                charge_product(&first, count, lift)?;
                product(&first, count, wide05::max_bits())?
            }
        }
        (MathModPow, [base, exponent, modulus]) => modpow(
            &natural(base)?,
            &natural(exponent)?,
            &natural(modulus)?,
            work,
        )?,
        (MathIsqrt, [x]) => natural(x)?.sqrt(),
        (MathPrime, [x]) => {
            return match classify(&natural(x)?, work, 0)? {
                PrimeStatus::Prime => Ok(V::Bool(true)),
                PrimeStatus::Composite => Ok(V::Bool(false)),
                PrimeStatus::Unknown => Err(THEORY_LIMIT),
            };
        }
        (MathFactor, [x]) => return factor_values(factor(natural(x)?)?),
        (MathDivisors, [x]) => {
            let n = natural(x)?;
            let factors = factor(n.clone())?;
            return divisor_values(n, factors);
        }
        (MathFib, [x]) => fibonacci(&natural(x)?, lift)?,
        _ => return Err("invalid checked number-theory operation"),
    };
    natural_value(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_test_is_only_a_proof_below_the_certified_bound() {
        // The first shared pseudoprime must not be accepted, while two
        // wide Mersenne primes can be proved with n−1 certificates.
        assert_ne!(
            classify(&BigUint::from(FIRST_PSEUDOPRIME), &Budget::new(), 0,)
                .unwrap_or(PrimeStatus::Unknown),
            PrimeStatus::Prime
        );
        for exponent in [89usize, 127] {
            let n = (BigUint::one() << exponent) - 1u8;
            assert_eq!(classify(&n, &Budget::new(), 0).unwrap(), PrimeStatus::Prime);
        }
    }

    #[test]
    fn near_word_limit_factors_are_sorted_and_certified() {
        let primes = [4_294_967_279u64, 4_294_967_291];
        let product = BigUint::from(primes[0]) * primes[1];
        assert_eq!(
            factor(product).unwrap(),
            primes.map(|p| (BigUint::from(p), 1))
        );
    }

    #[test]
    fn huge_prime_power_cannot_force_an_unbounded_output_list() {
        let n = BigUint::one() << (COLLECTION_LIMIT + 1);
        let factors = factor(n).unwrap();
        assert!(factor_values(factors).is_err());
    }
}
