//! The GPUs Windows reports, so Settings can say which one a GPU model would use. The
//! model itself runs through DirectML ([`super::onnx::Device::Gpu`]); this only reads
//! adapter names and memory sizes through DXGI, and never touches note data.

use serde::Serialize;

/// Dedicated video memory from which an adapter counts as a separate (discrete) GPU.
/// Integrated GPUs report little or none: the laptop's Intel Arc reported 128 MB.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
const DISCRETE_MIN_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    pub name: String,
    pub dedicated_bytes: u64,
    /// Has its own memory: the kind of GPU the fp16 model was measured fast on (#105).
    /// An integrated GPU indexed only about 1.4x faster than the CPU.
    pub discrete: bool,
}

/// The hardware adapters, the software renderer left out. Empty off Windows, or when
/// DXGI can't be asked.
pub fn adapters() -> Vec<Adapter> {
    imp::adapters()
}

/// The adapter a GPU model should run on: the discrete one with the most memory, if any.
pub fn best(adapters: &[Adapter]) -> Option<&Adapter> {
    adapters
        .iter()
        .filter(|a| a.discrete)
        .max_by_key(|a| a.dedicated_bytes)
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn adapter(name: String, dedicated_bytes: u64) -> Adapter {
    Adapter {
        name,
        dedicated_bytes,
        discrete: dedicated_bytes >= DISCRETE_MIN_BYTES,
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1,
    };

    use super::Adapter;

    pub fn adapters() -> Vec<Adapter> {
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for i in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
                break;
            };
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let end = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..end])
                .trim()
                .to_string();
            found.push(super::adapter(name, desc.DedicatedVideoMemory as u64));
        }
        found
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn adapters() -> Vec<super::Adapter> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_discrete_gpu_with_the_most_memory_is_chosen() {
        let list = vec![
            adapter("Intel(R) Arc(TM) Graphics".into(), 128 << 20),
            adapter("NVIDIA GeForce RTX 4060 Ti".into(), 8 << 30),
            adapter("Other".into(), 2 << 30),
        ];
        assert!(!list[0].discrete);
        assert_eq!(best(&list).unwrap().name, "NVIDIA GeForce RTX 4060 Ti");
        assert_eq!(
            best(&list[..1]),
            None,
            "an integrated GPU alone isn't offered"
        );
    }
}
