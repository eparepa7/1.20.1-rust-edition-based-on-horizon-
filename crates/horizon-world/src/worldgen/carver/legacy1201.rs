//! Version-isolated configured-carver inputs. These are not 26.3 mask carvers.
//! Geometry has its own float-indexed trig; ordered terrain application remains separate.
mod geometry;
pub mod trig;
use crate::worldgen::{
    DecodeError, anchor::VerticalAnchor, density::identifier, height_provider::HeightProvider,
    value_provider::FloatProvider,
};
use crate::{
    chunk::BlockState,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

fn field<'a>(value: &'a Value, name: &'static str) -> Result<&'a Value, DecodeError> {
    value.get(name).ok_or(DecodeError::Field(name))
}
fn scalar(value: &Value) -> Result<f32, DecodeError> {
    let number: f32 = serde_json::from_value(value.clone())?;
    if !number.is_finite() {
        return Err(DecodeError::Field("1.20.1 finite float"));
    }
    Ok(number)
}

/// 1.20.1 FloatProvider dispatch stores the provider's fields in `value`.
/// The current provider evaluator is reused only after explicit format decoding.
pub fn float_provider(value: &Value) -> Result<FloatProvider, DecodeError> {
    if value.is_number() {
        let value = scalar(value)?;
        return Ok(FloatProvider::Constant(if value == 0. {
            0.
        } else {
            value
        }));
    }
    let kind = identifier(
        field(value, "type")?
            .as_str()
            .ok_or(DecodeError::Field("type"))?,
    )?;
    let payload = field(value, "value")?;
    let mut fields = payload
        .as_object()
        .cloned()
        .ok_or(DecodeError::Field("1.20.1 float value"))?;
    fields.insert("type".into(), Value::String(kind));
    FloatProvider::from_json(&Value::Object(fields).to_string())
}

fn anchor(value: &Value) -> Result<VerticalAnchor, DecodeError> {
    let fields = value
        .as_object()
        .ok_or(DecodeError::Field("1.20.1 anchor"))?;
    if fields.len() != 1
        || !fields
            .keys()
            .all(|key| matches!(key.as_str(), "absolute" | "above_bottom" | "below_top"))
    {
        return Err(DecodeError::Field("1.20.1 anchor"));
    }
    VerticalAnchor::from_json(&value.to_string())
}

fn height_provider(value: &Value) -> Result<HeightProvider, DecodeError> {
    fn has_new_anchor(value: &Value) -> bool {
        match value {
            Value::Object(fields) => {
                fields.contains_key("relative_to_sea_level") || fields.values().any(has_new_anchor)
            }
            Value::Array(values) => values.iter().any(has_new_anchor),
            _ => false,
        }
    }
    if has_new_anchor(value) {
        return Err(DecodeError::Field("1.20.1 height anchor"));
    }
    HeightProvider::from_json(&value.to_string())
}

/// The four debug states are ordered air, water, lava, barrier.
#[derive(Clone, Debug)]
pub struct Legacy1201DebugSettings {
    pub enabled: bool,
    pub states: [BlockStateId; 4],
}
impl Legacy1201DebugSettings {
    fn decode(value: Option<&Value>, registry: &BlockRegistry) -> Result<Self, DecodeError> {
        let acacia = registry
            .default_state("minecraft:acacia_button")
            .ok_or(DecodeError::Field("debug default"))?;
        let Some(value) = value else {
            let mut states = [acacia; 4];
            for (index, name) in [
                (1, "minecraft:candle"),
                (2, "minecraft:orange_stained_glass"),
                (3, "minecraft:glass"),
            ] {
                states[index] = registry
                    .default_state(name)
                    .ok_or(DecodeError::Field("debug default"))?;
            }
            return Ok(Self {
                enabled: false,
                states,
            });
        };
        if !value.is_object() {
            return Err(DecodeError::Field("debug_settings"));
        }
        // CarverDebugSettings.CODEC defaults every omitted state to DEFAULT.airState.
        let mut states = [acacia; 4];
        for (index, name) in ["air_state", "water_state", "lava_state", "barrier_state"]
            .iter()
            .enumerate()
        {
            if let Some(raw) = value.get(name) {
                let name = identifier(
                    field(raw, "Name")?
                        .as_str()
                        .ok_or(DecodeError::Field("Name"))?,
                )?;
                let default = registry
                    .default_state(&name)
                    .ok_or(DecodeError::Field("debug block"))?;
                let mut descriptor: BlockState = registry.state(default).unwrap().clone();
                if let Some(properties) = raw.get("Properties") {
                    let properties = properties
                        .as_object()
                        .ok_or(DecodeError::Field("Properties"))?;
                    for (key, value) in properties {
                        descriptor.properties.insert(
                            key.clone(),
                            value
                                .as_str()
                                .ok_or(DecodeError::Field("debug property"))?
                                .into(),
                        );
                    }
                }
                // Strictly reject unsupported properties rather than guessing a state.
                states[index] = registry
                    .resolve(&descriptor)
                    .map_err(|e| DecodeError::Registry(e.to_string()))?;
            }
        }
        let enabled = value
            .get("debug_mode")
            .map(|v| v.as_bool().ok_or(DecodeError::Field("debug_mode")))
            .transpose()?
            .unwrap_or(false);
        Ok(Self { enabled, states })
    }
}

#[derive(Clone)]
pub struct Legacy1201CarverConfig<'r> {
    registry: &'r BlockRegistry,
    pub probability: f32,
    pub y: HeightProvider,
    pub y_scale: FloatProvider,
    pub lava_level: VerticalAnchor,
    pub debug: Legacy1201DebugSettings,
    replaceable: BTreeSet<String>,
}
impl Legacy1201CarverConfig<'_> {
    /// WorldCarver.canReplaceBlock: membership of this configuration's holder set.
    pub fn can_replace(&self, state: BlockStateId) -> Result<bool, DecodeError> {
        let state = self
            .registry
            .state(state)
            .ok_or(DecodeError::Field("carver state"))?;
        Ok(self.replaceable.contains(&state.name))
    }
}

#[derive(Clone, Debug)]
pub enum Legacy1201CarverShape {
    Cave {
        nether: bool,
        horizontal: FloatProvider,
        vertical: FloatProvider,
        floor: FloatProvider,
    },
    Canyon {
        rotation: FloatProvider,
        distance: FloatProvider,
        thickness: FloatProvider,
        width_smoothness: i32,
        horizontal: FloatProvider,
        vertical_default: f32,
        vertical_center: f32,
    },
}

#[derive(Clone)]
pub struct Legacy1201Carver<'r> {
    pub kind: String,
    pub config: Legacy1201CarverConfig<'r>,
    pub shape: Legacy1201CarverShape,
}
impl<'r> Legacy1201Carver<'r> {
    /// ConfiguredWorldCarver.DIRECT_CODEC / CarverConfiguration.CODEC (1.20.1).
    /// Tag sets must already have been resolved against this block registry.
    pub fn from_json(
        json: &str,
        registry: &'r BlockRegistry,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        let definition: Value = serde_json::from_str(json)?;
        let kind = identifier(
            field(&definition, "type")?
                .as_str()
                .ok_or(DecodeError::Field("type"))?,
        )?;
        let raw = field(&definition, "config")?;
        let provider = |name| float_provider(field(raw, name)?);
        let probability = scalar(field(raw, "probability")?)?;
        if !(0.0..=1.0).contains(&probability) {
            return Err(DecodeError::Field("carver probability"));
        }
        let replaceable = field(raw, "replaceable")?;
        let names = if let Some(tag) = replaceable.as_str().and_then(|s| s.strip_prefix('#')) {
            tags.get(&identifier(tag)?)
                .cloned()
                .ok_or(DecodeError::Field("missing 1.20.1 replaceable tag"))?
        } else {
            let entries = replaceable
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![replaceable.clone()]);
            entries
                .iter()
                .map(|v| identifier(v.as_str().ok_or(DecodeError::Field("replaceable holder"))?))
                .collect::<Result<Vec<_>, _>>()?
        };
        for name in &names {
            if registry.default_state(name).is_none() {
                return Err(DecodeError::Registry(format!(
                    "unknown replaceable block {name}"
                )));
            }
        }
        let config = Legacy1201CarverConfig {
            registry,
            probability,
            y: height_provider(field(raw, "y")?)?,
            y_scale: provider("yScale")?,
            lava_level: anchor(field(raw, "lava_level")?)?,
            debug: Legacy1201DebugSettings::decode(raw.get("debug_settings"), registry)?,
            replaceable: names.into_iter().collect(),
        };
        let shape = match kind.as_str() {
            "minecraft:cave" | "minecraft:nether_cave" => {
                let floor = provider("floor_level")?;
                // ConstantFloat.getMaxValue in 1.20.1 is value + 1, even
                // though sampling a constant never draws from the RNG.
                let max = match floor {
                    FloatProvider::Constant(value) => value + 1.,
                    _ => floor.bounds()[1],
                };
                if floor.bounds()[0] < -1. || max > 1. {
                    return Err(DecodeError::Field("carver floor"));
                }
                Legacy1201CarverShape::Cave {
                    nether: kind == "minecraft:nether_cave",
                    horizontal: provider("horizontal_radius_multiplier")?,
                    vertical: provider("vertical_radius_multiplier")?,
                    floor,
                }
            }
            "minecraft:canyon" => {
                let shape = field(raw, "shape")?;
                let shape_provider = |name| float_provider(field(shape, name)?);
                let width = field(shape, "width_smoothness")?
                    .as_i64()
                    .filter(|v| (0..=i64::from(i32::MAX)).contains(v))
                    .ok_or(DecodeError::Field("width_smoothness"))?;
                Legacy1201CarverShape::Canyon {
                    rotation: provider("vertical_rotation")?,
                    distance: shape_provider("distance_factor")?,
                    thickness: shape_provider("thickness")?,
                    width_smoothness: width as i32,
                    horizontal: shape_provider("horizontal_radius_factor")?,
                    vertical_default: scalar(field(shape, "vertical_radius_default_factor")?)?,
                    vertical_center: scalar(field(shape, "vertical_radius_center_factor")?)?,
                }
            }
            _ => return Err(DecodeError::Field("1.20.1 carver type")),
        };
        Ok(Self {
            kind,
            config,
            shape,
        })
    }
}

/// Separate AIR / LIQUID lists; ordering and repeated entries are significant.
pub struct Legacy1201BiomeCarvers<'r> {
    pub air: Vec<Arc<Legacy1201Carver<'r>>>,
    pub liquid: Vec<Arc<Legacy1201Carver<'r>>>,
}
pub struct Legacy1201CarverCatalog<'r> {
    pub definitions: BTreeMap<String, Arc<Legacy1201Carver<'r>>>,
}
impl<'r> Legacy1201CarverCatalog<'r> {
    pub fn from_json(
        definitions: &BTreeMap<String, String>,
        registry: &'r BlockRegistry,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        if definitions.len() > 8192 {
            return Err(DecodeError::Field("carver registry size"));
        }
        let mut result = BTreeMap::new();
        for (name, json) in definitions {
            let name = identifier(name)?;
            if result
                .insert(
                    name,
                    Arc::new(Legacy1201Carver::from_json(json, registry, tags)?),
                )
                .is_some()
            {
                return Err(DecodeError::Field("duplicate carver name"));
            }
        }
        Ok(Self {
            definitions: result,
        })
    }
    /// Only named vanilla holder lists are supported here. Tagged/inline sets fail closed.
    pub fn biome_steps(&self, json: &str) -> Result<Legacy1201BiomeCarvers<'r>, DecodeError> {
        let value: Value = serde_json::from_str(json)?;
        let object = value
            .as_object()
            .ok_or(DecodeError::Field("1.20.1 carving steps"))?;
        if object
            .keys()
            .any(|k| !matches!(k.as_str(), "air" | "liquid"))
        {
            return Err(DecodeError::Field("unknown carving step"));
        }
        let list = |step| -> Result<Vec<Arc<Legacy1201Carver<'r>>>, DecodeError> {
            let Some(entries) = object.get(step) else {
                return Ok(Vec::new());
            };
            let singleton;
            let entries = if let Some(entries) = entries.as_array() {
                entries.as_slice()
            } else {
                singleton = [entries.clone()];
                &singleton
            };
            if entries.len() > 1024 {
                return Err(DecodeError::Field("biome carver count"));
            }
            entries
                .iter()
                .map(|entry| {
                    let name = identifier(
                        entry
                            .as_str()
                            .ok_or(DecodeError::Field("named carver holder"))?,
                    )?;
                    self.definitions
                        .get(&name)
                        .cloned()
                        .ok_or_else(|| DecodeError::Registry(format!("unknown carver {name}")))
                })
                .collect()
        };
        Ok(Legacy1201BiomeCarvers {
            air: list("air")?,
            liquid: list("liquid")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::Tag;

    fn registry() -> BlockRegistry {
        let names = [
            "minecraft:air",
            "minecraft:stone",
            "minecraft:acacia_button",
            "minecraft:candle",
            "minecraft:orange_stained_glass",
            "minecraft:glass",
        ];
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            (
                "states",
                Tag::List(
                    names
                        .iter()
                        .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                        .collect(),
                ),
            ),
            (
                "defaults",
                Tag::compound(
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| (*name, Tag::Int(i as i32))),
                ),
            ),
        ]))
        .unwrap()
    }
    fn definition() -> Value {
        serde_json::json!({"type":"minecraft:cave", "config": {
            "probability":0.15, "y":{"absolute":10}, "yScale":1.0, "lava_level":{"above_bottom":8},
            "replaceable":"#test:replaceable", "horizontal_radius_multiplier":1.0,
            "vertical_radius_multiplier":1.0, "floor_level":-0.7,
        }})
    }
    fn tags() -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([("test:replaceable".into(), vec!["minecraft:stone".into()])])
    }

    #[test]
    fn replacement_is_bound_to_the_decoding_registry_and_missing_tags_fail() {
        let registry = registry();
        let text = definition().to_string();
        let carver = Legacy1201Carver::from_json(&text, &registry, &tags()).unwrap();
        assert!(carver.config.can_replace(BlockStateId(1)).unwrap());
        assert!(!carver.config.can_replace(BlockStateId(0)).unwrap());
        assert!(carver.config.can_replace(BlockStateId(100)).is_err());
        assert!(Legacy1201Carver::from_json(&text, &registry, &BTreeMap::new()).is_err());
    }
    #[test]
    fn absent_and_empty_debug_objects_have_different_vanilla_defaults() {
        let registry = registry();
        let mut value = definition();
        let absent = Legacy1201Carver::from_json(&value.to_string(), &registry, &tags()).unwrap();
        value["config"]["debug_settings"] = serde_json::json!({});
        let empty = Legacy1201Carver::from_json(&value.to_string(), &registry, &tags()).unwrap();
        assert_eq!(
            absent.config.debug.states,
            [
                BlockStateId(2),
                BlockStateId(3),
                BlockStateId(4),
                BlockStateId(5)
            ]
        );
        assert_eq!(empty.config.debug.states, [BlockStateId(2); 4]);
    }
    #[test]
    fn floor_constant_uses_legacy_maximum_and_zero_is_positive() {
        let registry = registry();
        let mut value = definition();
        value["config"]["floor_level"] = Value::from(0.5);
        assert!(Legacy1201Carver::from_json(&value.to_string(), &registry, &tags()).is_err());
        value["config"]["floor_level"] = Value::from(0.0);
        assert!(Legacy1201Carver::from_json(&value.to_string(), &registry, &tags()).is_ok());
        let FloatProvider::Constant(zero) = float_provider(&serde_json::json!(-0.0)).unwrap()
        else {
            panic!("constant");
        };
        assert_eq!(zero.to_bits(), 0);
    }
    #[test]
    fn steps_preserve_repeats_and_reject_unknown_or_tagged_entries() {
        let registry = registry();
        let definitions = BTreeMap::from([("test:cave".into(), definition().to_string())]);
        let catalog = Legacy1201CarverCatalog::from_json(&definitions, &registry, &tags()).unwrap();
        let steps = catalog
            .biome_steps(r#"{"air":["test:cave","test:cave"],"liquid":"test:cave"}"#)
            .unwrap();
        assert_eq!(steps.air.len(), 2);
        assert_eq!(steps.liquid.len(), 1);
        assert!(Arc::ptr_eq(&steps.air[0], &steps.air[1]));
        for json in [
            r#"{"air":["test:missing"]}"#,
            r##"{"air":"#test:set"}"##,
            r#"{"future":[]}"#,
            "[]",
        ] {
            assert!(catalog.biome_steps(json).is_err(), "{json}");
        }
    }
    #[test]
    fn newer_float_layout_and_sea_level_anchors_are_not_silently_accepted() {
        assert!(
            float_provider(
                &serde_json::json!({"type":"uniform","min_inclusive":0.0,"max_exclusive":1.0})
            )
            .is_err()
        );
        assert!(height_provider(&serde_json::json!({"relative_to_sea_level":0})).is_err());
        assert!(anchor(&serde_json::json!({"relative_to_sea_level":0})).is_err());
        assert!(anchor(&serde_json::json!({"absolute":0,"above_bottom":0})).is_err());
    }
    #[test]
    fn normalized_catalog_aliases_cannot_replace_each_other() {
        let registry = registry();
        let text = definition().to_string();
        let definitions = BTreeMap::from([
            ("cave".into(), text.clone()),
            ("minecraft:cave".into(), text),
        ]);
        assert!(Legacy1201CarverCatalog::from_json(&definitions, &registry, &tags()).is_err());
    }
}
