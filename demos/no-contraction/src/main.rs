//! Authored CPU metadata demonstration. No GPU or game-data dependency.
use std::{env, fs, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn words(bytes: &[u8]) -> Result<Vec<u32>> {
    if bytes.len() % 4 != 0 {
        return Err("SPIR-V byte length is not word aligned".into());
    }
    let result: Vec<_> = bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    if result.len() < 5 || result[0] != 0x07230203 {
        return Err("Invalid SPIR-V header".into());
    }
    Ok(result)
}
fn instructions(words: &[u32]) -> Result<Vec<&[u32]>> {
    let mut out = Vec::new();
    let mut pos = 5;
    while pos < words.len() {
        let n = (words[pos] >> 16) as usize;
        if n == 0 || n > words.len() - pos {
            return Err("Invalid SPIR-V instruction extent".into());
        }
        out.push(&words[pos..pos + n]);
        pos += n;
    }
    Ok(out)
}
fn controlled_producers(words: &[u32]) -> Result<Vec<u32>> {
    let instructions = instructions(words)?;
    let mut out = Vec::new();
    for d in &instructions {
        if d[0] & 65535 == 71 && d.len() >= 3 && d[2] == 42 {
            if d.len() != 3 {
                return Err("NoContraction has unexpected operands".into());
            }
            let producer = instructions
                .iter()
                .find(|i| i.len() >= 3 && i[2] == d[1] && matches!(i[0] & 65535, 129 | 133))
                .ok_or("Decoration is not on this demo's FAdd/FMul result")?;
            out.push(producer[0] & 65535);
        }
    }
    out.sort();
    Ok(out)
}
fn convert(input: &Path, output: &Path) -> Result<(Vec<u32>, Vec<u32>)> {
    let bytes = fs::read(input)?;
    let before = controlled_producers(&words(&bytes)?)?;
    let module = naga::front::spv::parse_u8_slice(&bytes, &Default::default())?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)?;
    let emitted = naga::back::spv::write_vec(&module, &info, &Default::default(), None)?;
    let after = controlled_producers(&emitted)?;
    fs::write(
        output,
        emitted
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )?;
    Ok((before, after))
}
fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 5 || !matches!(args[4].as_str(), "lost" | "preserved") {
        return Err(
            "usage: no-contraction-demo CONTROLLED.spv UNMARKED.spv NEW_OUTPUT_DIR lost|preserved"
                .into(),
        );
    }
    // Integer proof: (1 + 2^-23)(1 - 2^-23) = 1 - 2^-46.
    let a = (1u128 << 23) + 1;
    let b = (1u128 << 24) - 2;
    assert_eq!(a * b, (1u128 << 47) - 2);
    // Distance to 1 is 2^-46; distance to the next lower binary32 is 2^-24 - 2^-46.
    assert!(1u128 < (1u128 << 22) - 1);
    let separate = 0x00000000u32;
    let fused_reference = 0x80000000u32 | ((127u32 - 46) << 23);
    assert_eq!(fused_reference, 0xa8800000);
    let out = Path::new(&args[3]);
    fs::create_dir(out)?;
    let input = [0x3f800001u32, 0x3f7ffffe, 0xbf800000, 0];
    fs::write(
        out.join("input.bin"),
        input
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )?;
    let controlled = convert(Path::new(&args[1]), &out.join("controlled-roundtrip.spv"))?;
    let unmarked = convert(Path::new(&args[2]), &out.join("unmarked-roundtrip.spv"))?;
    assert_eq!(controlled.0, vec![129, 133]);
    assert!(unmarked.0.is_empty() && unmarked.1.is_empty());
    let expected = if args[4] == "lost" {
        vec![]
    } else {
        vec![129, 133]
    };
    assert_eq!(
        controlled.1, expected,
        "Observed metadata did not match the explicitly selected expectation"
    );
    let report = format!(
        "{{\n  \"gpu_executed\": false,\n  \"expectation\": \"{}\",\n  \"controlled_before\": {:?},\n  \"controlled_after\": {:?},\n  \"separate_expected_bits\": \"{:08x}\",\n  \"ideal_fused_reference_bits\": \"{:08x}\",\n  \"unmarked_gpu_difference_required\": false\n}}\n",
        args[4], controlled.0, controlled.1, separate, fused_reference
    );
    fs::write(out.join("cpu-result.json"), &report)?;
    print!("{report}");
    Ok(())
}
