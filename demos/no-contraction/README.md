# NoContraction: a small runtime-input example

This authored compute shader reads three binary32 operands from a 16-byte storage buffer and writes one binary32 result. `controlled.comp` declares the multiply and add results `precise`; `unmarked.comp` differs only by removing those two qualifiers. No game assets are used.

Input words, little endian: `3f800001 3f7ffffe bf800000 00000000`.
These are **a = 1 + 2^-23**, **b = 1 - 2^-23**, **c = -1**, plus padding.
The exact product is **1 - 2^-46**. Separately rounding the multiply and then the add to binary32 using round-to-nearest, ties-to-even gives **+0 (`00000000`)**. A correctly rounded fused reference gives **-2^-46 (`a8800000`)**.

The unmarked expression is permitted to remain separate: **equal GPU outputs are a valid outcome**, not evidence that the reproduction failed. This example does not claim WGSL `fma` must be fused or require every driver to contract an unmarked expression.

The deterministic defect demonstration is metadata: compile the controlled source to SPIR-V, inspect the two `NoContraction` producer decorations, then parse/re-emit with unpatched and patched Naga. The unpatched importer discards them; the patched path must preserve the restrictions or explicitly reject an unsupported backend. Actual GPU numeric measurements require a separately qualified finite runner and must report backend, device, compiler, flags, and all raw output bits.

## Run the CPU metadata check

Requirements: Rust/Cargo and official Khronos glslang on PATH (validated here with glslang 16.6.0). No game compiler, game data or graphics device is used. From a full checkout of this demonstration branch:

```sh
sh demos/no-contraction/run-cpu.sh lost
```

This compiles both authored shaders, generates `input.bin`, validates and roundtrips both modules through the checkout's Naga, and writes `cpu-result.json`. `lost` is the explicit expectation for the unpatched base. For the fixed case, copy this small demonstration directory into a separate checkout of the fix revision and run the same command with `preserved`. Naga is a local path dependency, so the harness never silently substitutes an unrelated registry build. The lock pins registry dependencies; `--offline` can be added to Cargo after they are cached.

On Windows without a shell, the equivalent commands from this directory are:

```text
glslang -V controlled.comp -o controlled.spv
glslang -V unmarked.comp -o unmarked.spv
cargo run --locked -- controlled.spv unmarked.spv results-old lost
```

Actual CPU qualification of this authored input: original producer opcodes `[FAdd, FMul]`; unpatched Naga output `[]`; held fixed candidate output `[FAdd, FMul]`. The unmarked roundtrip has no controls in both cases. Original and roundtripped modules pass `spirv-val`. These are metadata results, not measured GPU arithmetic. The harness checks an integer proof of the exact reference values; it never infers GPU behavior from a host compiler's floating-point evaluation.

The fix branch contains no demonstration files. An optional direct Vulkan measurement is documented below; it does not re-import emitted SPIR-V through Naga.


## Optional native Vulkan measurement

The separate `native` crate uses ash to submit the exact six SPIR-V modules to Vulkan, bypassing Naga re-import. Requirements: Vulkan 1.1 compute device, host-visible coherent memory, official Khronos validation layer and debug-utils. Run the CPU checks in separate old/fixed checkouts first and validate all six modules with official `spirv-val --target-env vulkan1.1`. This small harness accepts only the authored fixture; it is not a sandbox or a general SPIR-V validator.

From this demonstration directory (replace OLD and FIXED with the CPU result directories):

```text
cargo run --locked --manifest-path native/Cargo.toml -- gpu-plan OLD/input.bin controlled.spv unmarked.spv OLD/controlled-roundtrip.spv OLD/unmarked-roundtrip.spv FIXED/controlled-roundtrip.spv FIXED/unmarked-roundtrip.spv native-results
```

`gpu-plan` validates the expected inputs without loading Vulkan. Change it to `gpu-run` for one actual execution; `native-results` must not exist. The native fence timeout is 30 seconds. Keep an outer process timeout and preserve failure output. The program requires validation rather than silently disabling it. Inspect `validation-final.json` after process termination, including teardown: any error or truncation invalidates the measurement. A zero process exit alone does not certify teardown validation. `input-audit.bin` must match the original input. `device.json`, all six raw `.bin` words and `result.json` record the actual observation.

On Windows, if stale global implicit overlay manifests break loader validation, the official loader supports process-local `VK_IMPLICIT_LAYER_PATH` pointing at an empty directory; this replaces implicit discovery, rather than suppressing callback errors. Keep the explicit validation layer discoverable (for example through `VK_LAYER_PATH`). This excludes all implicit overlays in that process and should be recorded in results. Do not change the registry or silently ignore validation errors. See the official [loader documentation](https://github.com/KhronosGroup/Vulkan-Loader/blob/v1.4.357/docs/LoaderLayerInterface.md).

One validated observation: NVIDIA GeForce RTX 5080, native Vulkan API 1.4.357, driver raw version 2604892160, Khronos validation layer 1.4.368, glslang 16.6.0, unchanged default Vulkan pipeline flags. Original-controlled and fixed-controlled produced `00000000`; old-controlled and all three unmarked cases produced `a8800000`. Shared input GPU copy matched; final validation had zero messages, errors or warnings. This is a single authored fixture/device observation. Unmarked contraction is optional: equal outputs on other implementations are valid. It does not establish all-backend or whole-renderer parity.
