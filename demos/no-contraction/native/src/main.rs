use std::env;
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

mod gpu;
fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    if !args
        .get(1)
        .is_some_and(|a| a == "gpu-plan" || a == "gpu-run")
    {
        return Err("Expected gpu-plan or gpu-run".into());
    }
    gpu::command(&args[1..])
}
