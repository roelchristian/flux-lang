use flux_lang::RuntimeOptions;

pub(super) fn runtime_options() -> Result<RuntimeOptions, String> {
    fn setting(name: &str) -> Result<Option<String>, String> {
        match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8")),
        }
    }
    fn number<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
        setting(name)?.map_or(Ok(default), |value| {
            value
                .parse()
                .map_err(|_| format!("{name} must be a positive decimal integer"))
        })
    }

    let mut options = RuntimeOptions::default();
    let limits = &mut options.resources;
    limits.max_rank = number("FLUX_MAX_RANK", limits.max_rank)?;
    limits.max_elements = number("FLUX_MAX_ELEMENTS", limits.max_elements)?;
    limits.max_memory = number("FLUX_MAX_MEMORY", limits.max_memory)?;
    limits.max_work = number("FLUX_MAX_WORK", limits.max_work)?;
    limits.max_exact_bits = number("FLUX_MAX_EXACT_BITS", limits.max_exact_bits)?;
    limits.max_numeric_iterations =
        number("FLUX_MAX_NUMERIC_ITERATIONS", limits.max_numeric_iterations)?;
    limits.max_depth = number("FLUX_MAX_DEPTH", limits.max_depth)?;
    if let Some(value) = setting("FLUX_OPTIMIZE")? {
        options.optimize = match value.as_str() {
            "0" => false,
            "1" => true,
            _ => return Err("FLUX_OPTIMIZE must be 0 or 1".into()),
        };
    }
    options.validate().map_err(|e| e.to_string())?;
    Ok(options)
}
