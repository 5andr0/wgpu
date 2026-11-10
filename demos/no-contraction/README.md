# NoContraction: a small runtime-input example

This authored compute shader reads three binary32 operands from a 16-byte storage buffer and writes one binary32 result. `controlled.comp` declares the multiply and add results `precise`; `unmarked.comp` differs only by removing those two qualifiers. No game assets are used.

Input words, little endian: `3f800001 3f7ffffe bf800000 00000000`.
These are **a = 1 + 2^-23**, **b = 1 - 2^-23**, **c = -1**, plus padding.
The exact product is **1 - 2^-46**. Separately rounding the multiply and then the add to binary32 using round-to-nearest, ties-to-even gives **+0 (`00000000`)**. A correctly rounded fused reference gives **-2^-46 (`a8800000`)**.

The unmarked expression is permitted to remain separate: **equal GPU outputs are a valid outcome**, not evidence that the reproduction failed. This example does not claim WGSL `fma` must be fused or require every driver to contract an unmarked expression.

The deterministic defect demonstration is metadata: compile the controlled source to SPIR-V, inspect the two `NoContraction` producer decorations, then parse/re-emit with unpatched and patched Naga. The unpatched importer discards them; the patched path must preserve the restrictions or explicitly reject an unsupported backend. Actual GPU numeric measurements require a separately qualified finite runner and must report backend, device, compiler, flags, and all raw output bits.

## Run using the bundled shaders

The `fixtures` directory contains six compiled SPIR-V shaders and the 16-byte input. `manifest.json` records their hashes, source/compiler provenance and original/unpatched/fixed epochs. No separate glslang/DXC executable, shader compiler download or vendored compiler source is needed. Rust/Cargo builds the CPU harness with Naga from this checkout, or the native harness with ash. Normal Cargo source dependencies may need fetching unless cached. Building the separate main wgpu workspace still follows its own upstream dependencies.

The original shaders were compiled with the already available official Khronos glslang 16.6.0; their bytes match the retained qualified fixture. All six bundled modules pass `spirv-val --target-env vulkan1.1`. The fixed roundtrip is retained **candidate11** output, identified by patch/source-inventory hashes in the manifest. It is not relabeled as a newly generated candidate15 result. No new GPU execution was performed for this packaging change.

From a full checkout of this demonstration branch, run the CPU metadata check without a graphics device:

```sh
sh demos/no-contraction/run-cpu.sh lost
```

This uses the bundled original modules, generates `input.bin`, validates and roundtrips both modules through the checkout's Naga, and writes `cpu-result.json`. `lost` is the explicit expectation for the unpatched base. For the fixed case, copy this small demonstration directory into a separate checkout of the fix revision and run the same command with `preserved`. Naga is a local path dependency, so the harness never silently substitutes an unrelated registry build. The lock pins registry dependencies; `--offline` can be added to Cargo after they are cached.

On Windows, from this directory:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 -Mode Verify
powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 -Mode Cpu -Expectation lost -OutputDirectory results-old
```

`Verify` only checks the seven fixture file hashes and lengths. The authored `controlled.comp`/`unmarked.comp` sources remain available for inspection. Optional local regeneration, separate from normal demo usage, uses an existing glslang installation:

```text
glslang -V controlled.comp -o regenerated-controlled.spv
glslang -V unmarked.comp -o regenerated-unmarked.spv
```

Actual CPU qualification of this authored input: original producer opcodes `[FAdd, FMul]`; unpatched Naga output `[]`; held fixed candidate output `[FAdd, FMul]`. The unmarked roundtrip has no controls in both cases. Original and roundtripped modules pass `spirv-val`. These are metadata results, not measured GPU arithmetic. The harness checks an integer proof of the exact reference values; it never infers GPU behavior from a host compiler's floating-point evaluation.

The fix branch contains no demonstration files. An optional direct Vulkan measurement is documented below; it does not re-import emitted SPIR-V through Naga.


## Compare the published branches

Start in an empty working directory. These shallow checkouts select the unpatched demo branch and the reviewed precision fix explicitly:

```sh
git clone --depth 1 --branch codex/no-contraction-demo https://github.com/5andr0/wgpu.git wgpu-demo
git clone --depth 1 --branch codex/naga-preserve-no-contraction https://github.com/5andr0/wgpu.git wgpu-fixed
mkdir -p wgpu-fixed/demos
cp -R wgpu-demo/demos/no-contraction wgpu-fixed/demos/
sh wgpu-demo/demos/no-contraction/run-cpu.sh lost
sh wgpu-fixed/demos/no-contraction/run-cpu.sh preserved
```

On Windows, use the same two clone commands, then:

```powershell
New-Item -ItemType Directory -Path .\wgpu-fixed\demos -Force | Out-Null
Copy-Item -Recurse .\wgpu-demo\demos\no-contraction .\wgpu-fixed\demos\
powershell -NoProfile -ExecutionPolicy Bypass -File .\wgpu-demo\demos\no-contraction\run.ps1 -Mode Cpu -Expectation lost -OutputDirectory .\results-old
powershell -NoProfile -ExecutionPolicy Bypass -File .\wgpu-fixed\demos\no-contraction\run.ps1 -Mode Cpu -Expectation preserved -OutputDirectory .\results-fixed
```

The output directories must be new. Both runs use the same authored source/input fixtures and each checkout's local Naga: the controlled roundtrip must lose its decorations on the demo branch and preserve them on the fix branch. The unmarked roundtrip must remain unmarked. These commands check metadata on the CPU, not GPU arithmetic.

Copying the directory does not replace the bundled `candidate11` fixtures. The CPU harness writes freshly emitted modules to its result directory; it does not overwrite or relabel those retained GPU fixtures.

## Optional native Vulkan measurement

The separate `native` crate uses ash to submit the exact six bundled SPIR-V modules to Vulkan, bypassing Naga re-import. Requirements: Vulkan 1.1 compute device, host-visible coherent memory, official Khronos validation layer and debug-utils. This small harness accepts only the authored fixture; it is not a sandbox or a general SPIR-V validator. No shader compiler is invoked.

From this demonstration directory:

```text
cargo run --locked --manifest-path native/Cargo.toml -- gpu-plan fixtures/input.bin fixtures/original-controlled.spv fixtures/original-unmarked.spv fixtures/old-controlled.spv fixtures/old-unmarked.spv fixtures/fixed-controlled.spv fixtures/fixed-unmarked.spv native-results
```

Windows path convenience: `.\run.ps1 -Mode GpuPlan -OutputDirectory native-results`; use `-Mode GpuRun` for actual execution. These modes only select the same bundled paths and operation.

`gpu-plan` validates the expected inputs without loading Vulkan. Change it to `gpu-run` for one actual execution; `native-results` must not exist. The native fence timeout is 30 seconds. Keep an outer process timeout and preserve failure output. The program requires validation rather than silently disabling it. Inspect `validation-final.json` after process termination, including teardown: any error or truncation invalidates the measurement. A zero process exit alone does not certify teardown validation. `input-audit.bin` must match the original input. `device.json`, all six raw `.bin` words and `result.json` record the actual observation.

On Windows, if stale global implicit overlay manifests break loader validation, the official loader supports process-local `VK_IMPLICIT_LAYER_PATH` pointing at an empty directory; this replaces implicit discovery, rather than suppressing callback errors. Keep the explicit validation layer discoverable (for example through `VK_LAYER_PATH`). This excludes all implicit overlays in that process and should be recorded in results. Do not change the registry or silently ignore validation errors. See the official [loader documentation](https://github.com/KhronosGroup/Vulkan-Loader/blob/v1.4.357/docs/LoaderLayerInterface.md).

One validated observation: NVIDIA GeForce RTX 5080, native Vulkan API 1.4.357, driver raw version 2604892160, Khronos validation layer 1.4.368, glslang 16.6.0, unchanged default Vulkan pipeline flags. Original-controlled and fixed-controlled produced `00000000`; old-controlled and all three unmarked cases produced `a8800000`. Shared input GPU copy matched; final validation had zero messages, errors or warnings. This is a single authored fixture/device observation. Unmarked contraction is optional: equal outputs on other implementations are valid. It does not establish all-backend or whole-renderer parity.
