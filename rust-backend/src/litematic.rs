use crate::schema::{default_structure, make_cell, Cell, FloorName, Structure};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const NBT_TAG_END: u8 = 0;
const NBT_TAG_BYTE: u8 = 1;
const NBT_TAG_SHORT: u8 = 2;
const NBT_TAG_INT: u8 = 3;
const NBT_TAG_LONG: u8 = 4;
const NBT_TAG_FLOAT: u8 = 5;
const NBT_TAG_DOUBLE: u8 = 6;
const NBT_TAG_BYTE_ARRAY: u8 = 7;
const NBT_TAG_STRING: u8 = 8;
const NBT_TAG_LIST: u8 = 9;
const NBT_TAG_COMPOUND: u8 = 10;
const NBT_TAG_INT_ARRAY: u8 = 11;
const NBT_TAG_LONG_ARRAY: u8 = 12;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockState {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct ParsedLitematic {
    pub region_name: String,
    pub size: (i32, i32, i32),
    pub palette: Vec<BlockState>,
    pub indices: Vec<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LitematicExportOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_repeat: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LitematicImportOptions {
    #[serde(default, deserialize_with = "deserialize_usize_from_value")]
    pub floor_y: usize,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_option_usize_from_value"
    )]
    pub fluid_y: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_usize_from_value")]
    pub z: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LitematicRegionInfo {
    pub name: String,
    pub size: [i32; 3],
    pub floor_y: usize,
    pub fluid_y: usize,
    pub z: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LitematicImportResult {
    pub ok: bool,
    pub structure: Structure,
    pub region: LitematicRegionInfo,
    #[serde(default)]
    pub unknown_blocks: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
enum NbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(u8, Vec<NbtValue>),
    Compound(BTreeMap<String, NbtValue>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl NbtValue {
    fn tag_id(&self) -> u8 {
        match self {
            Self::Byte(_) => NBT_TAG_BYTE,
            Self::Short(_) => NBT_TAG_SHORT,
            Self::Int(_) => NBT_TAG_INT,
            Self::Long(_) => NBT_TAG_LONG,
            Self::Float(_) => NBT_TAG_FLOAT,
            Self::Double(_) => NBT_TAG_DOUBLE,
            Self::ByteArray(_) => NBT_TAG_BYTE_ARRAY,
            Self::String(_) => NBT_TAG_STRING,
            Self::List(_, _) => NBT_TAG_LIST,
            Self::Compound(_) => NBT_TAG_COMPOUND,
            Self::IntArray(_) => NBT_TAG_INT_ARRAY,
            Self::LongArray(_) => NBT_TAG_LONG_ARRAY,
        }
    }

    fn as_compound(&self) -> Option<&BTreeMap<String, NbtValue>> {
        match self {
            Self::Compound(value) => Some(value),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[NbtValue]> {
        match self {
            Self::List(_, values) => Some(values),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<i32> {
        match self {
            Self::Byte(value) => Some(*value as i32),
            Self::Short(value) => Some(*value as i32),
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    fn as_long_array(&self) -> Option<&[i64]> {
        match self {
            Self::LongArray(values) => Some(values),
            _ => None,
        }
    }
}

struct NbtReader<R> {
    reader: R,
}

impl<R: Read> NbtReader<R> {
    fn new(reader: R) -> Self {
        Self { reader }
    }

    fn named_root(&mut self) -> Result<(String, NbtValue), String> {
        let tag = self.read_u8()?;
        if tag != NBT_TAG_COMPOUND {
            return Err(format!(
                "Expected root compound tag {}, got {}.",
                NBT_TAG_COMPOUND, tag
            ));
        }
        let name = self.read_string()?;
        let payload = self.payload(tag)?;
        Ok((name, payload))
    }

    fn payload(&mut self, tag: u8) -> Result<NbtValue, String> {
        match tag {
            NBT_TAG_END => Err("Unexpected end tag payload.".to_string()),
            NBT_TAG_BYTE => Ok(NbtValue::Byte(self.read_i8()?)),
            NBT_TAG_SHORT => Ok(NbtValue::Short(self.read_i16()?)),
            NBT_TAG_INT => Ok(NbtValue::Int(self.read_i32()?)),
            NBT_TAG_LONG => Ok(NbtValue::Long(self.read_i64()?)),
            NBT_TAG_FLOAT => Ok(NbtValue::Float(self.read_f32()?)),
            NBT_TAG_DOUBLE => Ok(NbtValue::Double(self.read_f64()?)),
            NBT_TAG_BYTE_ARRAY => {
                let len = self.read_i32_len()?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.read_i8()?);
                }
                Ok(NbtValue::ByteArray(values))
            }
            NBT_TAG_STRING => Ok(NbtValue::String(self.read_string()?)),
            NBT_TAG_LIST => {
                let item_tag = self.read_u8()?;
                let len = self.read_i32_len()?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.payload(item_tag)?);
                }
                Ok(NbtValue::List(item_tag, values))
            }
            NBT_TAG_COMPOUND => {
                let mut compound = BTreeMap::new();
                loop {
                    let item_tag = self.read_u8()?;
                    if item_tag == NBT_TAG_END {
                        break;
                    }
                    let name = self.read_string()?;
                    let value = self.payload(item_tag)?;
                    compound.insert(name, value);
                }
                Ok(NbtValue::Compound(compound))
            }
            NBT_TAG_INT_ARRAY => {
                let len = self.read_i32_len()?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.read_i32()?);
                }
                Ok(NbtValue::IntArray(values))
            }
            NBT_TAG_LONG_ARRAY => {
                let len = self.read_i32_len()?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.read_i64()?);
                }
                Ok(NbtValue::LongArray(values))
            }
            other => Err(format!("Unsupported NBT tag type {}.", other)),
        }
    }

    fn read_exact<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut buf = [0_u8; N];
        self.reader
            .read_exact(&mut buf)
            .map_err(|error| format!("Failed reading NBT payload: {error}"))?;
        Ok(buf)
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        Ok(self.read_exact::<1>()?[0])
    }

    fn read_i8(&mut self) -> Result<i8, String> {
        Ok(i8::from_be_bytes(self.read_exact::<1>()?))
    }

    fn read_i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_be_bytes(self.read_exact::<2>()?))
    }

    fn read_i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.read_exact::<4>()?))
    }

    fn read_i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.read_exact::<8>()?))
    }

    fn read_f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_be_bytes(self.read_exact::<4>()?))
    }

    fn read_f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_be_bytes(self.read_exact::<8>()?))
    }

    fn read_string(&mut self) -> Result<String, String> {
        let len = u16::from_be_bytes(self.read_exact::<2>()?) as usize;
        let mut buf = vec![0_u8; len];
        self.reader
            .read_exact(&mut buf)
            .map_err(|error| format!("Failed reading NBT string: {error}"))?;
        String::from_utf8(buf).map_err(|error| format!("Invalid UTF-8 in NBT string: {error}"))
    }

    fn read_i32_len(&mut self) -> Result<usize, String> {
        let len = self.read_i32()?;
        usize::try_from(len).map_err(|_| format!("Negative or invalid NBT length {}.", len))
    }
}

pub fn parse_litematic(data: &[u8]) -> Result<ParsedLitematic, String> {
    let raw = maybe_gunzip(data)?;
    let (_, root) = NbtReader::new(Cursor::new(raw)).named_root()?;
    let root = root
        .as_compound()
        .ok_or_else(|| "Litematic root payload is not a compound.".to_string())?;
    let regions = root
        .get("Regions")
        .and_then(NbtValue::as_compound)
        .ok_or_else(|| "No Regions compound found in litematic.".to_string())?;
    let (region_name, region) = regions
        .iter()
        .next()
        .ok_or_else(|| "No region entries found in litematic.".to_string())?;
    let region = region
        .as_compound()
        .ok_or_else(|| "Region payload is not a compound.".to_string())?;
    let size = vector_to_tuple(region.get("Size"), (0, 0, 0));
    let palette_values = region
        .get("BlockStatePalette")
        .and_then(NbtValue::as_list)
        .ok_or_else(|| "Region has no BlockStatePalette.".to_string())?;
    let mut palette = Vec::with_capacity(palette_values.len());
    for value in palette_values {
        palette.push(block_state_from_nbt(value));
    }
    if palette.is_empty() {
        return Err("Region has no BlockStatePalette.".to_string());
    }
    let ax = abs_dim(size.0)?;
    let ay = abs_dim(size.1)?;
    let az = abs_dim(size.2)?;
    let volume = ax
        .checked_mul(ay)
        .and_then(|value| value.checked_mul(az))
        .ok_or_else(|| "Region volume overflow.".to_string())?;
    let block_states = region
        .get("BlockStates")
        .and_then(NbtValue::as_long_array)
        .unwrap_or(&[]);
    let indices = unpack_palette_indices(block_states, volume, palette.len());
    Ok(ParsedLitematic {
        region_name: region_name.clone(),
        size,
        palette,
        indices,
    })
}

pub fn export_litematic(
    structure: &Structure,
    options: Option<LitematicExportOptions>,
) -> Result<Vec<u8>, String> {
    let options = options.unwrap_or_default();
    let prefix = normalize_cells(&structure.prefix);
    let cycle = normalize_cells(&structure.cycle);
    let cycle_repeat = options.cycle_repeat.unwrap_or(64).clamp(1, 10_000);

    let mut cells = Vec::with_capacity(prefix.len() + cycle.len().saturating_mul(cycle_repeat));
    cells.extend(prefix);
    if cycle.is_empty() {
        cells.extend(cycle);
    } else {
        for _ in 0..cycle_repeat {
            cells.extend(cycle.iter().cloned());
        }
    }
    if cells.is_empty() {
        return Err("Structure has no cells to export.".to_string());
    }

    let width = i32::try_from(cells.len()).map_err(|_| "Structure is too wide to export.".to_string())?;
    let height = 2_i32;
    let length = 1_i32;

    let mut palette = vec![BlockState {
        name: "minecraft:air".to_string(),
        properties: BTreeMap::new(),
    }];
    let mut palette_index = BTreeMap::new();
    palette_index.insert(block_state_key(&palette[0]), 0_usize);

    let mut indices = Vec::with_capacity(cells.len() * height as usize * length as usize);
    let mut non_air = 0_i32;
    for y in 0..height {
        for _z in 0..length {
            for cell in &cells {
                let state = litematic_state_for_cell(cell, y == 0);
                if state.name != "minecraft:air" {
                    non_air += 1;
                }
                let key = block_state_key(&state);
                let palette_id = if let Some(index) = palette_index.get(&key) {
                    *index
                } else {
                    let index = palette.len();
                    palette.push(state);
                    palette_index.insert(key, index);
                    index
                };
                indices.push(palette_id);
            }
        }
    }

    let block_states = pack_palette_indices(&indices, palette.len());
    let now_ms = current_unix_time_millis()?;
    let name = options
        .name
        .or_else(|| structure.name.clone())
        .unwrap_or_else(|| "waterway".to_string());

    let mut metadata = BTreeMap::new();
    metadata.insert("Name".to_string(), NbtValue::String(name));
    metadata.insert(
        "Author".to_string(),
        NbtValue::String("item-waterway-solver-rust".to_string()),
    );
    metadata.insert(
        "Description".to_string(),
        NbtValue::String("Generated from item-waterway-solver structure.".to_string()),
    );
    metadata.insert("RegionCount".to_string(), NbtValue::Int(1));
    metadata.insert(
        "EnclosingSize".to_string(),
        vec3i_compound(width, height, length),
    );
    metadata.insert(
        "TotalVolume".to_string(),
        NbtValue::Int(width.saturating_mul(height).saturating_mul(length)),
    );
    metadata.insert("TotalBlocks".to_string(), NbtValue::Int(non_air));
    metadata.insert("TimeCreated".to_string(), NbtValue::Long(now_ms));
    metadata.insert("TimeModified".to_string(), NbtValue::Long(now_ms));

    let mut region = BTreeMap::new();
    region.insert("Position".to_string(), vec3i_compound(0, 0, 0));
    region.insert("Size".to_string(), vec3i_compound(width, height, length));
    region.insert(
        "BlockStatePalette".to_string(),
        NbtValue::List(
            NBT_TAG_COMPOUND,
            palette
                .iter()
                .map(block_state_to_nbt)
                .collect::<Vec<NbtValue>>(),
        ),
    );
    region.insert("BlockStates".to_string(), NbtValue::LongArray(block_states));
    region.insert(
        "Entities".to_string(),
        NbtValue::List(NBT_TAG_COMPOUND, Vec::new()),
    );
    region.insert(
        "TileEntities".to_string(),
        NbtValue::List(NBT_TAG_COMPOUND, Vec::new()),
    );
    region.insert(
        "PendingBlockTicks".to_string(),
        NbtValue::List(NBT_TAG_COMPOUND, Vec::new()),
    );
    region.insert(
        "PendingFluidTicks".to_string(),
        NbtValue::List(NBT_TAG_COMPOUND, Vec::new()),
    );

    let mut regions = BTreeMap::new();
    regions.insert("waterway".to_string(), NbtValue::Compound(region));

    let mut root = BTreeMap::new();
    root.insert("Version".to_string(), NbtValue::Int(5));
    root.insert("MinecraftDataVersion".to_string(), NbtValue::Int(2730));
    root.insert("Metadata".to_string(), NbtValue::Compound(metadata));
    root.insert("Regions".to_string(), NbtValue::Compound(regions));

    let raw = write_nbt_named_root("Litematica", &NbtValue::Compound(root))?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&raw)
        .map_err(|error| format!("Failed to write gzip payload: {error}"))?;
    encoder
        .finish()
        .map_err(|error| format!("Failed to finish gzip payload: {error}"))
}

pub fn import_litematic(
    data: &[u8],
    options: LitematicImportOptions,
) -> Result<LitematicImportResult, String> {
    let parsed = parse_litematic(data)?;
    let [sx, sy, sz] = [parsed.size.0, parsed.size.1, parsed.size.2];
    let ax = abs_dim(sx)?;
    let ay = abs_dim(sy)?;
    let az = abs_dim(sz)?;
    if ax == 0 || ay == 0 || az == 0 {
        return Err("Region size must be non-zero.".to_string());
    }

    let floor_y = options.floor_y.min(ay - 1);
    let fluid_default = (floor_y + 1).min(ay - 1);
    let fluid_y = options.fluid_y.unwrap_or(fluid_default).min(ay - 1);
    let lane_z = options.z.min(az - 1);

    let mut cells = Vec::with_capacity(ax);
    let mut unknown_blocks = BTreeMap::new();
    for x in 0..ax {
        let floor_block = block_at(&parsed, x, floor_y, lane_z);
        let fluid_block = block_at(&parsed, x, fluid_y, lane_z);
        let prev_fluid = x.checked_sub(1).map(|nx| block_at(&parsed, nx, fluid_y, lane_z));
        let next_fluid = if x + 1 < ax {
            Some(block_at(&parsed, x + 1, fluid_y, lane_z))
        } else {
            None
        };

        for block in [&floor_block, &fluid_block] {
            if !is_known_block(block.name.as_str()) {
                *unknown_blocks.entry(block.name.clone()).or_insert(0) += 1;
            }
        }

        cells.push(cell_from_blocks(
            &floor_block,
            &fluid_block,
            next_fluid.as_ref(),
            prev_fluid.as_ref(),
        ));
    }

    let mut structure = default_structure();
    structure.name = Some(format!("litematic:{}", parsed.region_name));
    structure.prefix = cells;
    structure.cycle = Vec::new();

    Ok(LitematicImportResult {
        ok: true,
        structure,
        region: LitematicRegionInfo {
            name: parsed.region_name,
            size: [sx, sy, sz],
            floor_y,
            fluid_y,
            z: lane_z,
        },
        unknown_blocks,
    })
}

pub fn unpack_palette_indices(
    long_array: &[i64],
    volume: usize,
    palette_size: usize,
) -> Vec<usize> {
    let bits = required_palette_bits(palette_size);
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << bits) - 1
    };
    let unsigned_longs = long_array.iter().map(|value| *value as u64).collect::<Vec<u64>>();
    let mut indices = Vec::with_capacity(volume);
    let mut bit_index = 0_usize;

    for _ in 0..volume {
        let long_index = bit_index / 64;
        let start_bit = bit_index % 64;
        let mut value = if long_index < unsigned_longs.len() {
            (unsigned_longs[long_index] >> start_bit) & mask
        } else {
            0
        };
        let spill = start_bit + bits;
        if spill > 64 && long_index + 1 < unsigned_longs.len() {
            let spill_bits = spill - 64;
            value |= (unsigned_longs[long_index + 1] & ((1_u64 << spill_bits) - 1))
                << (bits - spill_bits);
        }
        indices.push(value as usize);
        bit_index += bits;
    }

    indices
}

pub fn pack_palette_indices(indices: &[usize], palette_size: usize) -> Vec<i64> {
    let bits = required_palette_bits(palette_size);
    let total_bits = indices.len().saturating_mul(bits);
    let long_len = ((total_bits + 63) / 64).max(1);
    let mut longs = vec![0_u64; long_len];

    for (offset, palette_index) in indices.iter().enumerate() {
        let bit_index = offset * bits;
        let long_index = bit_index / 64;
        let start_bit = bit_index % 64;
        let value = *palette_index as u64;

        if long_index < longs.len() {
            longs[long_index] |= value << start_bit;
        }
        let spill = start_bit + bits;
        if spill > 64 && long_index + 1 < longs.len() {
            let spill_bits = spill - 64;
            longs[long_index + 1] |= value >> (bits - spill_bits);
        }
    }

    longs.into_iter().map(|value| value as i64).collect()
}

fn deserialize_usize_from_value<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum UsizeValue {
        Integer(usize),
        Text(String),
    }

    match UsizeValue::deserialize(deserializer)? {
        UsizeValue::Integer(value) => Ok(value),
        UsizeValue::Text(text) => text
            .trim()
            .parse::<usize>()
            .map_err(serde::de::Error::custom),
    }
}

fn deserialize_option_usize_from_value<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OptionUsizeValue {
        Integer(usize),
        Text(String),
        Null(()),
    }

    match OptionUsizeValue::deserialize(deserializer)? {
        OptionUsizeValue::Integer(value) => Ok(Some(value)),
        OptionUsizeValue::Text(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<usize>()
                    .map(Some)
                    .map_err(serde::de::Error::custom)
            }
        }
        OptionUsizeValue::Null(_) => Ok(None),
    }
}

fn maybe_gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    if !data.starts_with(&[0x1f, 0x8b]) {
        return Ok(data.to_vec());
    }
    let mut decoder = GzDecoder::new(Cursor::new(data));
    let mut raw = Vec::new();
    decoder
        .read_to_end(&mut raw)
        .map_err(|error| format!("Failed to gunzip litematic payload: {error}"))?;
    Ok(raw)
}

fn normalize_cells(cells: &[Cell]) -> Vec<Cell> {
    cells.iter().cloned().map(Cell::normalized).collect()
}

fn litematic_state_for_cell(cell: &Cell, is_floor_layer: bool) -> BlockState {
    let normalized = cell.clone().normalized();
    if is_floor_layer {
        return BlockState {
            name: litematic_floor_name(&normalized.floor).to_string(),
            properties: BTreeMap::new(),
        };
    }
    let amount = normalized.canonical_amount();
    if amount == 0 || normalized.canonical_surface().is_none() {
        return BlockState {
            name: "minecraft:air".to_string(),
            properties: BTreeMap::new(),
        };
    }

    let level = if amount >= 8 {
        0
    } else {
        (8_u8.saturating_sub(amount)).clamp(1, 7)
    };
    let mut properties = BTreeMap::new();
    properties.insert("level".to_string(), level.to_string());
    BlockState {
        name: "minecraft:water".to_string(),
        properties,
    }
}

fn litematic_floor_name(floor: &FloorName) -> &'static str {
    match floor.as_str().trim().to_ascii_lowercase().as_str() {
        "packed_ice" | "ice" | "frosted_ice" => "minecraft:packed_ice",
        "blue_ice" => "minecraft:blue_ice",
        "slime" | "slime_block" => "minecraft:slime_block",
        _ => "minecraft:glass",
    }
}

fn block_state_key(state: &BlockState) -> String {
    let mut key = state.name.clone();
    if !state.properties.is_empty() {
        key.push('{');
        let mut first = true;
        for (name, value) in &state.properties {
            if !first {
                key.push(',');
            }
            first = false;
            key.push_str(name);
            key.push('=');
            key.push_str(value);
        }
        key.push('}');
    }
    key
}

fn current_unix_time_millis() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?;
    i64::try_from(duration.as_millis()).map_err(|_| "Unix time overflow.".to_string())
}

fn write_nbt_named_root(name: &str, payload: &NbtValue) -> Result<Vec<u8>, String> {
    if !matches!(payload, NbtValue::Compound(_)) {
        return Err("Litematic root payload must be a compound.".to_string());
    }
    let mut out = Vec::new();
    out.push(NBT_TAG_COMPOUND);
    write_nbt_string(&mut out, name)?;
    write_nbt_payload(&mut out, payload)?;
    Ok(out)
}

fn write_nbt_payload<W: Write>(writer: &mut W, value: &NbtValue) -> Result<(), String> {
    match value {
        NbtValue::Byte(number) => writer
            .write_all(&number.to_be_bytes())
            .map_err(|error| format!("Failed writing NBT byte: {error}"))?,
        NbtValue::Short(number) => writer
            .write_all(&number.to_be_bytes())
            .map_err(|error| format!("Failed writing NBT short: {error}"))?,
        NbtValue::Int(number) => writer
            .write_all(&number.to_be_bytes())
            .map_err(|error| format!("Failed writing NBT int: {error}"))?,
        NbtValue::Long(number) => writer
            .write_all(&number.to_be_bytes())
            .map_err(|error| format!("Failed writing NBT long: {error}"))?,
        NbtValue::Float(number) => writer
            .write_all(&number.to_be_bytes())
            .map_err(|error| format!("Failed writing NBT float: {error}"))?,
        NbtValue::Double(number) => writer
            .write_all(&number.to_be_bytes())
            .map_err(|error| format!("Failed writing NBT double: {error}"))?,
        NbtValue::ByteArray(values) => {
            let len = i32::try_from(values.len())
                .map_err(|_| "NBT byte array length overflow.".to_string())?;
            writer
                .write_all(&len.to_be_bytes())
                .map_err(|error| format!("Failed writing NBT byte array length: {error}"))?;
            for value in values {
                writer
                    .write_all(&value.to_be_bytes())
                    .map_err(|error| format!("Failed writing NBT byte array item: {error}"))?;
            }
        }
        NbtValue::String(text) => write_nbt_string(writer, text)?,
        NbtValue::List(item_tag, values) => {
            writer
                .write_all(&[*item_tag])
                .map_err(|error| format!("Failed writing NBT list tag: {error}"))?;
            let len =
                i32::try_from(values.len()).map_err(|_| "NBT list length overflow.".to_string())?;
            writer
                .write_all(&len.to_be_bytes())
                .map_err(|error| format!("Failed writing NBT list length: {error}"))?;
            for value in values {
                write_nbt_payload(writer, value)?;
            }
        }
        NbtValue::Compound(entries) => {
            for (name, value) in entries {
                writer
                    .write_all(&[value.tag_id()])
                    .map_err(|error| format!("Failed writing NBT compound tag: {error}"))?;
                write_nbt_string(writer, name)?;
                write_nbt_payload(writer, value)?;
            }
            writer
                .write_all(&[NBT_TAG_END])
                .map_err(|error| format!("Failed writing NBT compound end tag: {error}"))?;
        }
        NbtValue::IntArray(values) => {
            let len = i32::try_from(values.len())
                .map_err(|_| "NBT int array length overflow.".to_string())?;
            writer
                .write_all(&len.to_be_bytes())
                .map_err(|error| format!("Failed writing NBT int array length: {error}"))?;
            for value in values {
                writer
                    .write_all(&value.to_be_bytes())
                    .map_err(|error| format!("Failed writing NBT int array item: {error}"))?;
            }
        }
        NbtValue::LongArray(values) => {
            let len = i32::try_from(values.len())
                .map_err(|_| "NBT long array length overflow.".to_string())?;
            writer
                .write_all(&len.to_be_bytes())
                .map_err(|error| format!("Failed writing NBT long array length: {error}"))?;
            for value in values {
                writer
                    .write_all(&value.to_be_bytes())
                    .map_err(|error| format!("Failed writing NBT long array item: {error}"))?;
            }
        }
    }
    Ok(())
}

fn write_nbt_string<W: Write>(writer: &mut W, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let len =
        u16::try_from(bytes.len()).map_err(|_| "NBT string length exceeds u16.".to_string())?;
    writer
        .write_all(&len.to_be_bytes())
        .map_err(|error| format!("Failed writing NBT string length: {error}"))?;
    writer
        .write_all(bytes)
        .map_err(|error| format!("Failed writing NBT string bytes: {error}"))
}

fn vec3i_compound(x: i32, y: i32, z: i32) -> NbtValue {
    let mut values = BTreeMap::new();
    values.insert("x".to_string(), NbtValue::Int(x));
    values.insert("y".to_string(), NbtValue::Int(y));
    values.insert("z".to_string(), NbtValue::Int(z));
    NbtValue::Compound(values)
}

fn block_state_to_nbt(state: &BlockState) -> NbtValue {
    let mut payload = BTreeMap::new();
    payload.insert("Name".to_string(), NbtValue::String(state.name.clone()));
    if !state.properties.is_empty() {
        let mut props = BTreeMap::new();
        for (name, value) in &state.properties {
            props.insert(name.clone(), NbtValue::String(value.clone()));
        }
        payload.insert("Properties".to_string(), NbtValue::Compound(props));
    }
    NbtValue::Compound(payload)
}

fn block_state_from_nbt(value: &NbtValue) -> BlockState {
    let mut state = BlockState {
        name: "minecraft:air".to_string(),
        properties: BTreeMap::new(),
    };
    let Some(compound) = value.as_compound() else {
        return state;
    };
    if let Some(name) = compound.get("Name").and_then(NbtValue::as_string) {
        state.name = name.to_string();
    }
    if let Some(props) = compound.get("Properties").and_then(NbtValue::as_compound) {
        for (name, value) in props {
            if let Some(value) = value.as_string() {
                state.properties.insert(name.clone(), value.to_string());
            }
        }
    }
    state
}

fn vector_to_tuple(value: Option<&NbtValue>, fallback: (i32, i32, i32)) -> (i32, i32, i32) {
    if let Some(compound) = value.and_then(NbtValue::as_compound) {
        let x = compound
            .get("x")
            .and_then(NbtValue::as_int)
            .unwrap_or(fallback.0);
        let y = compound
            .get("y")
            .and_then(NbtValue::as_int)
            .unwrap_or(fallback.1);
        let z = compound
            .get("z")
            .and_then(NbtValue::as_int)
            .unwrap_or(fallback.2);
        return (x, y, z);
    }
    if let Some(list) = value.and_then(NbtValue::as_list) {
        if list.len() >= 3 {
            let x = list[0].as_int().unwrap_or(fallback.0);
            let y = list[1].as_int().unwrap_or(fallback.1);
            let z = list[2].as_int().unwrap_or(fallback.2);
            return (x, y, z);
        }
    }
    fallback
}

fn required_palette_bits(palette_size: usize) -> usize {
    let mut value = palette_size.max(1) - 1;
    let mut bits = 0_usize;
    while value > 0 {
        bits += 1;
        value >>= 1;
    }
    bits.max(2)
}

fn abs_dim(value: i32) -> Result<usize, String> {
    usize::try_from(value.unsigned_abs()).map_err(|_| format!("Invalid litematic dimension {}.", value))
}

fn block_at(parsed: &ParsedLitematic, x: usize, y: usize, z: usize) -> BlockState {
    let ax = abs_dim(parsed.size.0).unwrap_or(0);
    let az = abs_dim(parsed.size.2).unwrap_or(0);
    let index = (y.saturating_mul(az).saturating_add(z))
        .saturating_mul(ax)
        .saturating_add(x);
    let palette_index = parsed.indices.get(index).copied().unwrap_or(0);
    parsed
        .palette
        .get(palette_index)
        .cloned()
        .unwrap_or_else(|| BlockState {
            name: "minecraft:air".to_string(),
            properties: BTreeMap::new(),
        })
}

fn short_block_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn is_known_block(name: &str) -> bool {
    matches!(
        short_block_name(name),
        "air"
            | "water"
            | "glow_lichen"
            | "stone_pressure_plate"
            | "glass"
            | "slime_block"
            | "slime"
            | "packed_ice"
            | "blue_ice"
            | "ice"
            | "frosted_ice"
            | "stone"
            | "normal"
    )
}

fn floor_from_block(name: &str) -> FloorName {
    let short = short_block_name(name);
    match short {
        "packed_ice" | "ice" | "frosted_ice" => FloorName::from("packed_ice"),
        "blue_ice" => FloorName::from("blue_ice"),
        "slime_block" | "slime" => FloorName::from("slime"),
        _ if short.contains("glass") => FloorName::from("glass"),
        _ if short.contains("stone")
            || matches!(short, "air" | "water" | "glow_lichen" | "stone_pressure_plate") =>
        {
            FloorName::from("stone")
        }
        _ => FloorName::from("normal"),
    }
}

fn cell_from_blocks(
    floor_block: &BlockState,
    fluid_block: &BlockState,
    next_fluid: Option<&BlockState>,
    prev_fluid: Option<&BlockState>,
) -> Cell {
    let floor = floor_from_block(&floor_block.name);
    let name = short_block_name(&fluid_block.name);
    let waterlogged = fluid_block
        .properties
        .get("waterlogged")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let is_water = name == "water" || waterlogged;
    if !is_water {
        return make_cell(None, 0, floor, None, Some(0));
    }
    if waterlogged && name != "water" {
        return make_cell(Some(8.0 / 9.0), 0, floor, None, Some(8));
    }

    let level_num = fluid_block
        .properties
        .get("level")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0);
    let amount = if level_num == 0 {
        8_u8
    } else {
        (8_i32.saturating_sub(level_num)).clamp(1, 8) as u8
    };
    let prev_amount = water_amount(prev_fluid);
    let next_amount = water_amount(next_fluid);
    let mut flow = 0_i8;
    if prev_amount.is_some() || next_amount.is_some() {
        let left = prev_amount.unwrap_or(amount);
        let right = next_amount.unwrap_or(amount);
        if amount > right {
            flow = 1;
        } else if amount > left {
            flow = -1;
        }
    }
    make_cell(Some(amount as f64 / 9.0), flow, floor, None, Some(amount))
}

fn water_amount(block: Option<&BlockState>) -> Option<u8> {
    let block = block?;
    let name = short_block_name(&block.name);
    if block
        .properties
        .get("waterlogged")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return Some(8);
    }
    if name != "water" {
        return None;
    }
    let level = block
        .properties
        .get("level")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0);
    Some(if level == 0 {
        8
    } else {
        (8_i32.saturating_sub(level)).clamp(1, 8) as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn import_options_accept_numeric_strings() {
        let options: LitematicImportOptions = serde_json::from_value(json!({
            "floorY": "0",
            "fluidY": "1",
            "z": "0",
        }))
        .expect("deserialize litematic import options");
        assert_eq!(options.floor_y, 0);
        assert_eq!(options.fluid_y, Some(1));
        assert_eq!(options.z, 0);
    }

    #[test]
    fn exported_litematic_can_be_imported_back() {
        let structure = default_structure();
        let bytes = export_litematic(
            &structure,
            Some(LitematicExportOptions {
                cycle_repeat: Some(2),
                name: Some("roundtrip".to_string()),
            }),
        )
        .expect("export litematic");

        let imported = import_litematic(
            &bytes,
            LitematicImportOptions {
                floor_y: 0,
                fluid_y: Some(1),
                z: 0,
            },
        )
        .expect("import exported litematic");

        assert!(imported.ok);
        assert_eq!(imported.region.size, [12, 2, 1]);
        assert_eq!(imported.structure.prefix.len(), 12);
        assert!(imported.structure.cycle.is_empty());
        assert!(imported.unknown_blocks.is_empty());
    }
}
