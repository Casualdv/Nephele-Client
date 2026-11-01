//! Vanilla 1.8.9 block models.
//!
//! 1.8 is the version that moved block shapes into data: `blockstates/<name>.json`
//! maps a block's state properties to a model, and `models/block/<name>.json`
//! describes the boxes (`elements`) and textures of that model, usually through a
//! `parent` chain. Loading them gives us stairs, slabs, fences, panes, doors and
//! torches for free instead of hand-coding every shape.
//!
//! This module only parses and resolves; meshing lives in `nephele-render`.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::atlas::AssetError;

/// The six faces a model element can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFace {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl ModelFace {
    pub const ALL: [ModelFace; 6] = [
        ModelFace::Down,
        ModelFace::Up,
        ModelFace::North,
        ModelFace::South,
        ModelFace::West,
        ModelFace::East,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            ModelFace::Down => "down",
            ModelFace::Up => "up",
            ModelFace::North => "north",
            ModelFace::South => "south",
            ModelFace::West => "west",
            ModelFace::East => "east",
        }
    }

    /// The direction this face points, as a block offset.
    pub const fn normal(self) -> [i32; 3] {
        match self {
            ModelFace::Down => [0, -1, 0],
            ModelFace::Up => [0, 1, 0],
            ModelFace::North => [0, 0, -1],
            ModelFace::South => [0, 0, 1],
            ModelFace::West => [-1, 0, 0],
            ModelFace::East => [1, 0, 0],
        }
    }

    /// Polygon winding in the element's local space (counter-clockwise from
    /// outside), matching vanilla's vertex order.
    pub const fn corners(self) -> [[f32; 3]; 4] {
        match self {
            ModelFace::Up => [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
            ModelFace::Down => [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ],
            ModelFace::North => [
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
            ModelFace::South => [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            ModelFace::West => [
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [0.0, 1.0, 0.0],
            ],
            ModelFace::East => [
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 1.0],
            ],
        }
    }

    /// Which corner of the element's UV rectangle each winding vertex takes.
    pub const fn uv_corners(self) -> [[f32; 2]; 4] {
        match self {
            ModelFace::Down => [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            ModelFace::Up => [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            ModelFace::North => [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            ModelFace::South => [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            ModelFace::West => [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            ModelFace::East => [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        }
    }

    pub fn from_name(name: &str) -> Option<ModelFace> {
        match name {
            "down" => Some(ModelFace::Down),
            "up" => Some(ModelFace::Up),
            "north" => Some(ModelFace::North),
            "south" => Some(ModelFace::South),
            "west" => Some(ModelFace::West),
            "east" => Some(ModelFace::East),
            _ => None,
        }
    }

    /// Index into `Element::faces`.
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// One box of a model, in block-local 0..16 units.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// Sparse: a face may be absent, which means "not drawn".
    pub faces: [Option<FaceSpec>; 6],
}

impl Element {
    pub fn face(&self, face: ModelFace) -> Option<&FaceSpec> {
        self.faces[face.index()].as_ref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceSpec {
    /// Texture-array layer name, e.g. `planks_oak`.
    pub texture: String,
    /// UV rectangle in 0..1 texture space.
    pub uv: [f32; 4],
    /// 0, 90, 180 or 270 degrees.
    pub rotation: u8,
    /// Face whose neighbouring block hides this one (`cullface`).
    pub cullface: Option<ModelFace>,
    /// 0 or 1: vanilla tints these faces with the grass/foliage colour.
    pub tintindex: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockModel {
    pub textures: HashMap<String, String>,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub model: String,
    pub x: i32,
    pub y: i32,
    pub uvlock: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Blockstate {
    /// Empty for blockstates that use `multipart` (fences, panes, ...).
    pub variants: HashMap<String, Variant>,
    pub multipart: bool,
}

/// Every model and blockstate shipped in `assets/minecraft`.
pub struct BlockModels {
    raw_models: HashMap<String, Value>,
    blockstates: HashMap<String, Blockstate>,
    resolved: HashMap<String, BlockModel>,
}

impl BlockModels {
    /// Load `blockstates/*.json` and `models/block/*.json` from the
    /// `assets/minecraft` directory.
    pub fn load(minecraft_dir: &Path) -> Result<Self, AssetError> {
        let blockstates_dir = minecraft_dir.join("blockstates");
        let models_dir = minecraft_dir.join("models/block");
        if !blockstates_dir.is_dir() || !models_dir.is_dir() {
            return Err(AssetError::MissingDirectory(
                minecraft_dir.display().to_string(),
            ));
        }

        let mut blockstates = HashMap::new();
        for (name, value) in read_json_dir(&blockstates_dir)? {
            blockstates.insert(name, parse_blockstate(&value));
        }

        let mut raw_models: HashMap<String, Value> = HashMap::new();
        for (name, value) in read_json_dir(&models_dir)? {
            raw_models.insert(format!("block/{name}"), value);
        }

        Ok(Self {
            raw_models,
            blockstates,
            resolved: HashMap::new(),
        })
    }

    pub fn blockstate(&self, name: &str) -> Option<&Blockstate> {
        self.blockstates.get(name)
    }

    pub fn blockstate_names(&self) -> impl Iterator<Item = &String> {
        self.blockstates.keys()
    }

    /// Resolve a model name (`"block/oak_stairs"` or `"oak_stairs"`), following
    /// its `parent` chain. Results are cached.
    pub fn model(&mut self, name: &str) -> Option<&BlockModel> {
        let key = if name.starts_with("block/") {
            name.to_string()
        } else {
            format!("block/{name}")
        };
        if !self.resolved.contains_key(&key) {
            let mut model = self.resolve(&key, 0)?;
            // Texture slots can be filled in by a child model, so references
            // are only resolved once the whole chain has been merged.
            resolve_model_textures(&mut model);
            self.resolved.insert(key.clone(), model);
        }
        self.resolved.get(&key)
    }

    fn resolve(&self, key: &str, depth: usize) -> Option<BlockModel> {
        if depth > 16 {
            return None;
        }
        let raw = self.raw_models.get(key)?;

        let mut model = match raw.get("parent").and_then(Value::as_str) {
            Some(parent) if !parent.starts_with("builtin/") => {
                let parent_key = if parent.starts_with("block/") {
                    parent.to_string()
                } else {
                    format!("block/{parent}")
                };
                self.resolve(&parent_key, depth + 1).unwrap_or_default()
            }
            _ => BlockModel::default(),
        };

        if let Some(textures) = raw.get("textures").and_then(Value::as_object) {
            for (slot, value) in textures {
                if let Some(texture) = value.as_str() {
                    model.textures.insert(slot.clone(), texture.to_string());
                }
            }
        }

        // A child replaces its parent's geometry when it defines its own.
        if let Some(elements) = raw.get("elements").and_then(Value::as_array) {
            model.elements = parse_elements(elements);
        }

        Some(model)
    }
}

fn read_json_dir(dir: &Path) -> Result<Vec<(String, Value)>, AssetError> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|_| AssetError::MissingDirectory(dir.display().to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|_| AssetError::NoTextures(format!("unreadable model {}", path.display())))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| AssetError::NoTextures(format!("{}: {err}", path.display())))?;
        out.push((stem.to_string(), value));
    }
    Ok(out)
}

fn parse_blockstate(value: &Value) -> Blockstate {
    let mut state = Blockstate::default();
    if let Some(variants) = value.get("variants").and_then(Value::as_object) {
        for (key, variant) in variants {
            // A variant value can be a list (random models); take the first.
            let variant = variant
                .as_array()
                .and_then(|list| list.first())
                .unwrap_or(variant);
            let Some(model) = variant.get("model").and_then(Value::as_str) else {
                continue;
            };
            state.variants.insert(
                key.clone(),
                Variant {
                    model: model.to_string(),
                    x: variant.get("x").and_then(Value::as_i64).unwrap_or(0) as i32,
                    y: variant.get("y").and_then(Value::as_i64).unwrap_or(0) as i32,
                    uvlock: variant
                        .get("uvlock")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
            );
        }
    }
    if value.get("multipart").is_some() {
        state.multipart = true;
    }
    state
}

fn parse_elements(elements: &[Value]) -> Vec<Element> {
    let mut out = Vec::new();
    for element in elements {
        let Some(from) = read_vec3(element.get("from")) else {
            continue;
        };
        let Some(to) = read_vec3(element.get("to")) else {
            continue;
        };
        let mut faces: [Option<FaceSpec>; 6] = Default::default();
        if let Some(map) = element.get("faces").and_then(Value::as_object) {
            for (name, spec) in map {
                let Some(face) = ModelFace::from_name(name) else {
                    continue;
                };
                let texture = spec
                    .get("texture")
                    .and_then(Value::as_str)
                    .unwrap_or("missing")
                    .to_string();
                let uv = spec
                    .get("uv")
                    .and_then(Value::as_array)
                    .and_then(|values| {
                        let nums: Vec<f32> = values
                            .iter()
                            .filter_map(|v| v.as_f64())
                            .map(|v| v as f32)
                            .collect();
                        if nums.len() == 4 {
                            Some([
                                nums[0] / 16.0,
                                nums[1] / 16.0,
                                nums[2] / 16.0,
                                nums[3] / 16.0,
                            ])
                        } else {
                            None
                        }
                    })
                    .unwrap_or([0.0, 0.0, 1.0, 1.0]);
                faces[face.index()] = Some(FaceSpec {
                    texture,
                    uv,
                    rotation: spec
                        .get("rotation")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        .rem_euclid(360) as u8,
                    cullface: spec
                        .get("cullface")
                        .and_then(Value::as_str)
                        .and_then(ModelFace::from_name),
                    tintindex: spec.get("tintindex").and_then(Value::as_i64).unwrap_or(-1) as i32,
                });
            }
        }
        out.push(Element { from, to, faces });
    }
    out
}

fn read_vec3(value: Option<&Value>) -> Option<[f32; 3]> {
    let array = value?.as_array()?;
    if array.len() != 3 {
        return None;
    }
    Some([
        array[0].as_f64()? as f32,
        array[1].as_f64()? as f32,
        array[2].as_f64()? as f32,
    ])
}

/// Resolve every `#slot` face reference against a model's texture map.
fn resolve_model_textures(model: &mut BlockModel) {
    for element in &mut model.elements {
        for face in element.faces.iter_mut().flatten() {
            face.texture = resolve_texture(&face.texture, &model.textures);
        }
    }
}

/// Follow `#slot` references until a concrete `blocks/name` is found, then
/// return the texture-array layer name.
fn resolve_texture(reference: &str, textures: &HashMap<String, String>) -> String {
    let mut current = reference.to_string();
    for _ in 0..16 {
        let Some(slot) = current.strip_prefix('#') else {
            break;
        };
        match textures.get(slot) {
            Some(next) => current = next.clone(),
            None => return "missing".to_string(),
        }
    }
    current
        .strip_prefix("blocks/")
        .unwrap_or(&current)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/vanilla/1.8.9/assets/minecraft")
    }

    fn library() -> BlockModels {
        BlockModels::load(&assets()).expect("vanilla models are extracted")
    }

    #[test]
    fn loads_every_blockstate_and_model() {
        let models = library();
        assert!(
            models.blockstates.len() > 300,
            "{} blockstates",
            models.blockstates.len()
        );
        assert!(
            models.raw_models.len() > 1000,
            "{} models",
            models.raw_models.len()
        );
    }

    #[test]
    fn resolves_a_parent_chain() {
        let mut models = library();
        // oak_stairs parents block/stairs, which defines the geometry.
        let model = models.model("oak_stairs").expect("oak_stairs");
        assert!(!model.elements.is_empty(), "geometry comes from the parent");
        assert!(
            model
                .elements
                .iter()
                .any(|e| e.face(ModelFace::Up).is_some()),
            "stairs have a top face"
        );
        let textures: Vec<&String> = model
            .elements
            .iter()
            .flat_map(|e| e.faces.iter().flatten())
            .map(|f| &f.texture)
            .collect();
        assert!(
            textures.iter().all(|t| *t == "planks_oak"),
            "textures resolve through #all: {textures:?}"
        );
    }

    #[test]
    fn slabs_are_half_height() {
        let mut models = library();
        let state = models.blockstate("stone_slab").expect("stone_slab").clone();
        let variant = state.variants.get("half=bottom").expect("bottom slab");
        let name = variant.model.clone();
        let model = models.model(&name).expect("slab model");
        assert_eq!(model.elements.len(), 1);
        let element = &model.elements[0];
        assert_eq!(element.from[1], 0.0);
        assert_eq!(element.to[1], 8.0, "a bottom slab is 8/16 tall");
    }

    #[test]
    fn stairs_blockstate_has_all_facings() {
        let models = library();
        let state = models.blockstate("oak_stairs").expect("oak_stairs");
        assert_eq!(state.variants.len(), 40, "4 facings x 2 halves x 5 shapes");
        assert!(state
            .variants
            .contains_key("facing=east,half=bottom,shape=straight"));
    }

    #[test]
    fn uv_is_normalised_and_rotations_are_valid() {
        let mut models = library();
        let model = models.model("oak_stairs").expect("oak_stairs");
        for element in &model.elements {
            for face in element.faces.iter().flatten() {
                assert!((0.0..=1.0).contains(&face.uv[0]));
                assert!((0.0..=1.0).contains(&face.uv[2]));
                assert_eq!(face.rotation % 90, 0);
            }
        }
    }
}
