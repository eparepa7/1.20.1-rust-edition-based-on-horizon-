//! Local 1.20.1 configured-carver decoding and scalar sampling, not path generation.
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        carver::legacy1201::{Legacy1201Carver, Legacy1201CarverCatalog, Legacy1201CarverShape},
        tags::{TagDefinitions, TagFile},
    },
};
use std::{collections::BTreeMap, error::Error, fs, path::Path};

fn bundle(root: &Path, name: &str) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        root.join(format!("{name}.json")),
    )?)?)
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and private bundle directory".into());
    }
    let registry = BlockRegistry::from_facts_json(fs::File::open(&args[0])?)?;
    let root = Path::new(&args[1]);
    let mut definitions = TagDefinitions::default();
    for (name, json) in bundle(root, "block_tags")? {
        definitions.add(&name, "1.20.1", TagFile::from_json(&json)?)?;
    }
    let built =
        definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    if !built.missing.is_empty() {
        return Err("unresolved block tags".into());
    }
    let parsed = bundle(root, "carvers")?
        .into_iter()
        .map(|(name, json)| {
            (
                name,
                Legacy1201Carver::from_json(&json, &registry, &built.tags),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let catalog = Legacy1201CarverCatalog {
        definitions: parsed
            .iter()
            .filter_map(|(name, value)| {
                value
                    .as_ref()
                    .ok()
                    .map(|carver| (name.clone(), std::sync::Arc::new(carver.clone())))
            })
            .collect(),
    };
    for (name, result) in &parsed {
        let Ok(carver) = result else {
            println!("E {name}");
            continue;
        };
        let config = &carver.config;
        println!(
            "C {name} {} {} {} {} {} {} {}",
            carver.kind,
            config.probability.to_bits(),
            u8::from(config.debug.enabled),
            config.debug.states[0].0,
            config.debug.states[1].0,
            config.debug.states[2].0,
            config.debug.states[3].0
        );
        for i in 0..registry.len() {
            println!(
                "B {name} {i} {}",
                u8::from(config.can_replace(BlockStateId(i as u32))?)
            );
        }
        for (context_index, (min, height)) in [(-64, 384), (0, 128), (-2032, 4064)]
            .into_iter()
            .enumerate()
        {
            let context = WorldGenerationContext::new(min, height, 63);
            println!(
                "L {name} {context_index} {}",
                config.lava_level.resolve(context)
            );
            for seed in [0, 1, -1, i64::MIN, i64::MAX, 0x12345678] {
                let mut random = LegacyRandom::new(seed);
                for index in 0..32 {
                    let start = random.next_float() <= config.probability;
                    let y = config.y.sample(&mut random, context)?;
                    let scale = config.y_scale.sample(&mut random);
                    let mut values = vec![scale];
                    match &carver.shape {
                        Legacy1201CarverShape::Cave {
                            horizontal,
                            vertical,
                            floor,
                            ..
                        } => {
                            values.extend([
                                horizontal.sample(&mut random),
                                vertical.sample(&mut random),
                                floor.sample(&mut random),
                            ]);
                        }
                        Legacy1201CarverShape::Canyon {
                            rotation,
                            distance,
                            thickness,
                            horizontal,
                            vertical_default,
                            vertical_center,
                            width_smoothness,
                        } => {
                            values.extend([
                                rotation.sample(&mut random),
                                distance.sample(&mut random),
                                thickness.sample(&mut random),
                                horizontal.sample(&mut random),
                                *vertical_default,
                                *vertical_center,
                                *width_smoothness as f32,
                            ]);
                        }
                    }
                    let bits = values
                        .iter()
                        .map(|v| v.to_bits().to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!(
                        "R {name} {context_index} {seed} {index} {} {y} {bits} {}",
                        u8::from(start),
                        random.next_long()
                    );
                }
            }
        }
    }
    let biomes = bundle(root, "biomes")?;
    for (name, json) in biomes {
        let value: serde_json::Value = serde_json::from_str(&json)?;
        let steps = catalog.biome_steps(&value["carvers"].to_string())?;
        for (step, carvers) in [("air", steps.air), ("liquid", steps.liquid)] {
            let names = carvers
                .iter()
                .map(|c| {
                    catalog
                        .definitions
                        .iter()
                        .find(|(_, v)| std::sync::Arc::ptr_eq(v, c))
                        .unwrap()
                        .0
                        .as_str()
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("S {name} {step} {names}");
        }
    }
    Ok(())
}
