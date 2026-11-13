//! Binary transport for 1.20.1 trig and geometry oracle comparisons.
use horizon_core::{
    carving::CarverOutput,
    position::ChunkPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    registry::BlockRegistry,
    worldgen::{
        anchor::WorldGenerationContext,
        carver::legacy1201::{Legacy1201CarverCatalog, trig},
        tags::{TagDefinitions, TagFile},
    },
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::{self, BufWriter, Write},
    path::Path,
};

fn bundle(root: &Path, name: &str) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        root.join(format!("{name}.json")),
    )?)?)
}
#[derive(Deserialize)]
struct Fixture {
    carver: String,
    seed: i64,
    target: [i32; 2],
    origin: [i32; 2],
    min: i32,
    height: i32,
    select_start: bool,
    upgrading: bool,
}
struct Trace<'a> {
    out: &'a mut dyn Write,
    minimum: i32,
    height: i32,
    upgrading: bool,
    debug: bool,
    seen: Vec<bool>,
    count: u32,
}
impl CarverOutput for Trace<'_> {
    type Error = io::Error;
    fn min_y(&self) -> i32 {
        self.minimum + 1
    }
    fn max_y(&self) -> i32 {
        self.minimum + self.height - 1 - if self.upgrading { 0 } else { 7 }
    }
    fn carve(&mut self, x: i32, y: i32, z: i32) -> io::Result<()> {
        if !(0..16).contains(&x) || !(0..16).contains(&z) || y <= self.min_y() || y > self.max_y() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid candidate coordinate",
            ));
        }
        let packed = ((y - self.minimum) << 8) | (z << 4) | x;
        if !self.seen[packed as usize] || self.debug {
            self.seen[packed as usize] = true;
            self.out.write_all(&packed.to_be_bytes())?;
            self.count += 1;
        }
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 3 && args[0] == "trig" {
        let angles: Vec<u32> = serde_json::from_slice(&fs::read(&args[1])?)?;
        let mut out = BufWriter::new(fs::File::create(&args[2])?);
        out.write_all(&0x4c435431u32.to_be_bytes())?;
        for value in trig::sine_table() {
            out.write_all(&value.to_bits().to_be_bytes())?;
        }
        for bits in angles {
            let angle = f32::from_bits(bits);
            out.write_all(&bits.to_be_bytes())?;
            out.write_all(&trig::sin(angle).to_bits().to_be_bytes())?;
            out.write_all(&trig::cos(angle).to_bits().to_be_bytes())?;
        }
        out.flush()?;
        return Ok(());
    }
    if args.len() != 5 || args[0] != "geometry" {
        return Err(
            "expected geometry facts bundles fixtures output, or trig angles output".into(),
        );
    }
    let registry = BlockRegistry::from_facts_json(fs::File::open(&args[1])?)?;
    let root = Path::new(&args[2]);
    let mut definitions = TagDefinitions::default();
    for (name, json) in bundle(root, "block_tags")? {
        definitions.add(&name, "1.20.1", TagFile::from_json(&json)?)?;
    }
    let built =
        definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    if !built.missing.is_empty() {
        return Err("unresolved block tags".into());
    }
    let catalog =
        Legacy1201CarverCatalog::from_json(&bundle(root, "carvers")?, &registry, &built.tags)?;
    let fixtures: Vec<Fixture> = serde_json::from_slice(&fs::read(&args[3])?)?;
    let mut out = BufWriter::new(fs::File::create(&args[4])?);
    out.write_all(&0x4c434731u32.to_be_bytes())?;
    for (index, f) in fixtures.into_iter().enumerate() {
        if !(1..=4064).contains(&f.height) || f.min.checked_add(f.height).is_none() {
            return Err("fixture height is out of range".into());
        }
        let carver = catalog.definitions.get(&f.carver).ok_or("unknown carver")?;
        let mut random = LegacyRandom::new(f.seed);
        let selected = !f.select_start || carver.is_start_chunk(&mut random);
        out.write_all(&(index as u32).to_be_bytes())?;
        out.write_all(&[u8::from(selected)])?;
        let mut trace = Trace {
            out: &mut out,
            minimum: f.min,
            height: f.height,
            upgrading: f.upgrading,
            debug: carver.config.debug.enabled,
            seen: vec![false; f.height as usize * 256],
            count: 0,
        };
        let result = selected
            && carver.carve_geometry(
                WorldGenerationContext::new(f.min, f.height, 63),
                &mut random,
                ChunkPos::new(f.target[0], f.target[1]),
                ChunkPos::new(f.origin[0], f.origin[1]),
                &mut trace,
            )?;
        let count = trace.count;
        out.write_all(&(-1i32).to_be_bytes())?;
        out.write_all(&count.to_be_bytes())?;
        out.write_all(&[u8::from(result)])?;
        out.write_all(&random.next_long().to_be_bytes())?;
    }
    out.flush()?;
    Ok(())
}
