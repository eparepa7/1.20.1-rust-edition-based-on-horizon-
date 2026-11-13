//! Compare 1.20.1 heightmap predicates using facts read from a local Java oracle.
use horizon_world::heightmap::{HeightmapType, Legacy1201HeightmapState};
use std::{
    error::Error,
    fs::File,
    io::{BufRead, BufReader},
};

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args_os().nth(1).ok_or("expected oracle output")?;
    for (expected_id, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 12 || fields[0].parse::<usize>()? != expected_id {
            return Err("invalid oracle row".into());
        }
        let bit = |i: usize| -> Result<bool, Box<dyn Error>> {
            match fields[i] {
                "0" => Ok(false),
                "1" => Ok(true),
                _ => Err("invalid boolean".into()),
            }
        };
        let state = Legacy1201HeightmapState {
            air: bit(1)?,
            air_block: bit(2)?,
            fluid: bit(3)?,
            blocks_motion: bit(4)?,
            leaves_block: bit(5)?,
        };
        print!(
            "{} {} {} {} {} {}",
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5]
        );
        for kind in HeightmapType::ALL {
            print!(" {}", u8::from(kind.is_opaque(state)));
        }
        println!();
    }
    Ok(())
}
