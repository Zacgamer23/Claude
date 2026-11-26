//! Procesamiento de audio en tiempo real: sin asignaciones tras la construcción.

#[cfg_attr(not(windows), allow(dead_code))]
pub mod drift;
pub mod jitter;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod resample;
