//! Device selection for the neural diffusion backend (v4.2.0+).
//!
//! Picks a CUDA device when the `neural-cuda` Cargo feature is on
//! AND a GPU is available at runtime, falls back to CPU otherwise.
//! This keeps the neural pathway runnable in three modes:
//!
//! 1. `cargo build --features neural` → CPU-only (no CUDA deps).
//! 2. `cargo build --features neural-cuda` on a machine with CUDA
//!    drivers + GPU → uses `Device::Cuda(0)`.
//! 3. `cargo build --features neural-cuda` on a machine without a
//!    GPU → gracefully falls back to CPU at runtime.
//!
//! All existing call sites that previously hardcoded `Device::Cpu`
//! should call [`preferred_device`] instead.

use candle_core::Device;

/// Return the preferred device for neural diffusion work.
///
/// With `neural-cuda` feature: probes `cuda_if_available(0)`, falls
/// back to CPU if no GPU is present. Without the feature: always CPU.
pub fn preferred_device() -> Device {
    #[cfg(feature = "neural-cuda")]
    {
        match Device::cuda_if_available(0) {
            Ok(d) => d,
            Err(_) => Device::Cpu,
        }
    }
    #[cfg(not(feature = "neural-cuda"))]
    {
        Device::Cpu
    }
}

/// Is CUDA actually being used at runtime?
///
/// Returns `true` only when the `neural-cuda` feature is compiled in
/// AND the machine has a working GPU. Useful for conditional logging
/// and feature-flag smoke tests.
pub fn cuda_available() -> bool {
    #[cfg(feature = "neural-cuda")]
    {
        matches!(preferred_device(), Device::Cuda(_))
    }
    #[cfg(not(feature = "neural-cuda"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_device_returns_something() {
        // Always returns a device — CPU at worst.
        let _device = preferred_device();
    }

    #[test]
    fn cuda_available_matches_feature() {
        // Without the cuda feature, always false.
        #[cfg(not(feature = "neural-cuda"))]
        assert!(!cuda_available());
    }
}
