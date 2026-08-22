#![allow(unused_variables, dead_code, dropping_references)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    tauri_build::build();
    generate_tl();
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FType {
    Flags,
    Int,
    Long,
    Double,
    Int128,
    Int256,
    Str,
    Bytes,
    Bool,    // boxed Bool (4 bytes ctor)
    True,    // flag bit, no wire data
    Object,  // nested TL object
    VecInt,
    VecLong,
    VecStr,
    VecBytes,
    VecObj,
}

#[derive(Debug, Clone)]
struct Field {
    name: String,
    ftype: FType,
    flag_field: Option<String>,
    flag_bit: Option<u32>,
}

#[derive(Debug, Clone)]
struct Constructor {
    name: String,
    id: u32,
    fields: Vec<Field>,
    result_type: String,
    is_function: bool,
}

fn generate_tl() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let schema_path = Path::new(&manifest_dir).join("schema.txt");
    if !schema_path.exists() { return; }
    println!("cargo:rerun-if-changed={}", schema_path.display());

    let schema = fs::read_to_string(&schema_path).expect("read schema.txt");
    let constructors = parse_schema(&schema);

    let out_dir = Path::new(&manifest_dir).join("src").join("mtproto");
    let out_path = out_dir.join("tl_gen.rs");
    let mut out = fs::File::create(&out_path).expect("create tl_gen.rs");

    write_header(&mut out);
    write_constants(&mut out, &constructors);
    write_ctor_name(&mut out, &constructors);
    write_field_descriptors(&mut out, &constructors);
    write_skip_engine(&mut out);
    write_method_builders(&mut out, &constructors);
    write_type_serializers(&mut out, &constructors);
    write_type_deserializers(&mut out, &constructors);
    write_trait_impls(&mut out, &constructors);
    write_method_parsers(&mut out, &constructors);
}

fn parse_schema(schema: &str) -> Vec<Constructor> {
    let mut constructors = Vec::new();
    let mut is_function = false;
    for line in schema.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") { continue; }
        if line == "---functions---" { is_function = true; continue; }
        if line == "---types---" { is_function = false; continue; }
        if !line.contains('#') || !line.contains('=') { continue; }
        if let Some(mut ctor) = parse_line(line) {
            ctor.is_function = is_function;
            constructors.push(ctor);
        }
    }
    // deduplicate: if the same ctor_id appears multiple times, keep only the last
    // occurrence (newer definition). also deduplicate by (result_type, name) to avoid
    // duplicate enum variants when a constructor is redefined with a new ctor_id.
    let mut seen_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut seen_names: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for ctor in constructors.into_iter().rev() {
        if seen_ids.contains(&ctor.id) { continue; }
        let key = (ctor.result_type.clone(), ctor.name.clone());
        if seen_names.contains(&key) { continue; }
        seen_ids.insert(ctor.id);
        seen_names.insert(key);
        deduped.push(ctor);
    }
    deduped.reverse();
    deduped
}

fn parse_line(line: &str) -> Option<Constructor> {
    let line = line.trim_end_matches(';').trim();
    let eq_pos = line.rfind('=')?;
    let result_type = line[eq_pos + 1..].trim().to_string();
    let left = line[..eq_pos].trim();
    let parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() { return None; }

    let name_id = parts[0];
    let hash_pos = name_id.find('#')?;
    let name = name_id[..hash_pos].to_string();
    let id = u32::from_str_radix(&name_id[hash_pos + 1..], 16).ok()?;

    let mut fields = Vec::new();
    for part in &parts[1..] {
        if part.starts_with('{') || part.starts_with('#') || part.starts_with('[') || part.starts_with(']') {
            continue;
        }
        if let Some(f) = parse_field(part) {
            fields.push(f);
        }
    }
    Some(Constructor { name, id, fields, result_type, is_function: false })
}

fn parse_field(s: &str) -> Option<Field> {
    let colon = s.find(':')?;
    let name = s[..colon].to_string();
    let type_str = &s[colon + 1..];

    let (flag_field, flag_bit, actual) = if let Some(q) = type_str.find('?') {
        let cond = &type_str[..q];
        let dot = cond.find('.')?;
        let ff = cond[..dot].to_string();
        let bit: u32 = cond[dot + 1..].parse().ok()?;
        (Some(ff), Some(bit), &type_str[q + 1..])
    } else {
        (None, None, type_str)
    };

    let ftype = parse_type(actual)?;
    Some(Field { name, ftype, flag_field, flag_bit })
}

fn parse_type(s: &str) -> Option<FType> {
    if s.starts_with("Vector<") || s.starts_with("vector<") {
        let inner = &s[7..s.len() - 1];
        return Some(match inner {
            "int" => FType::VecInt,
            "long" => FType::VecLong,
            "string" => FType::VecStr,
            "bytes" => FType::VecBytes,
            _ => FType::VecObj,
        });
    }
    Some(match s {
        "#" => FType::Flags,
        "int" => FType::Int,
        "long" => FType::Long,
        "double" => FType::Double,
        "int128" => FType::Int128,
        "int256" => FType::Int256,
        "string" => FType::Str,
        "bytes" => FType::Bytes,
        "Bool" => FType::Bool,
        "true" | "True" => FType::True,
        _ => FType::Object,
    })
}

fn to_const_name(name: &str) -> String {
    let mut result = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch == '.' { result.push('_'); prev_lower = false; continue; }
        if ch.is_uppercase() && prev_lower { result.push('_'); }
        result.push(ch.to_ascii_uppercase());
        prev_lower = ch.is_lowercase();
    }
    result
}

fn write_header(out: &mut fs::File) {
    writeln!(out, "// auto-generated from schema.txt — do not edit").unwrap();
    writeln!(out, "#![allow(dead_code, unused_variables, unused_imports, non_snake_case, unused_mut, unused_parens)]").unwrap();
    writeln!(out, "").unwrap();
    writeln!(out, "use byteorder::{{LittleEndian, ReadBytesExt}};").unwrap();
    writeln!(out, "use std::io::{{Cursor, Read}};").unwrap();
    writeln!(out, "use std::collections::HashMap;").unwrap();
    writeln!(out, "use super::tl::{{deserialize_bytes, deserialize_string}};").unwrap();
    writeln!(out, "").unwrap();
}

fn write_constants(out: &mut fs::File, ctors: &[Constructor]) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    writeln!(out, "// --- constructor IDs ---").unwrap();
    for ctor in ctors {
        let cn = to_const_name(&ctor.name);
        if let Some(&prev_id) = seen.get(&cn) {
            if prev_id != ctor.id {
                let suffixed = format!("{}_{:08X}", cn, ctor.id);
                if !seen.contains_key(&suffixed) {
                    seen.insert(suffixed.clone(), ctor.id);
                    writeln!(out, "pub const {}: u32 = {:#010x};", suffixed, ctor.id).unwrap();
                }
            }
        } else {
            seen.insert(cn.clone(), ctor.id);
            writeln!(out, "pub const {}: u32 = {:#010x};", cn, ctor.id).unwrap();
        }
    }
    writeln!(out, "").unwrap();
}

fn write_ctor_name(out: &mut fs::File, ctors: &[Constructor]) {
    writeln!(out, "pub fn ctor_name(id: u32) -> &'static str {{").unwrap();
    writeln!(out, "    match id {{").unwrap();
    let mut seen_ids = std::collections::HashSet::new();
    for ctor in ctors {
        if seen_ids.insert(ctor.id) {
            writeln!(out, "        {:#010x} => \"{}\",", ctor.id, ctor.name).unwrap();
        }
    }
    writeln!(out, "        _ => \"unknown\",").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();
}

// field descriptor encoding:
// each field is encoded as a u16:
//   bits 0-3: type (0=int,1=long,2=double,3=str,4=bytes,5=obj,6=flags,
//                   7=bool,8=vec_int,9=vec_long,10=vec_str,11=vec_bytes,
//                   12=vec_obj,13=int128,14=int256,15=true/skip)
//   bits 4-8: flag_bit (0-31), only meaningful if bit 15 is set
//   bit 9-10: flag_field_index (0=flags, 1=flags2, 2=flags3)
//   bit 15: conditional (1=has condition)
//
// we store descriptors as &[u16] per constructor in a static array,
// then a HashMap<u32, (usize, usize)> maps ctor_id -> (offset, len) into the array.
//
// but for simplicity and compile speed, we use a phf-free approach:
// generate a sorted array of (ctor_id, field_slice_offset, field_count)
// and binary search at runtime.

fn encode_ftype(ft: FType) -> u16 {
    match ft {
        FType::Int => 0,
        FType::Long => 1,
        FType::Double => 2,
        FType::Str => 3,
        FType::Bytes => 4,
        FType::Object => 5,
        FType::Flags => 6,
        FType::Bool => 7,
        FType::VecInt => 8,
        FType::VecLong => 9,
        FType::VecStr => 10,
        FType::VecBytes => 11,
        FType::VecObj => 12,
        FType::Int128 => 13,
        FType::Int256 => 14,
        FType::True => 15,
    }
}

fn flag_field_index(name: &str) -> u16 {
    match name {
        "flags" => 0,
        "flags2" => 1,
        _ => 2, // flags3 or other
    }
}

fn write_field_descriptors(out: &mut fs::File, ctors: &[Constructor]) {
    // encode all fields into a flat u16 array
    let mut all_fields: Vec<u16> = Vec::new();
    // (ctor_id, offset_into_all_fields, field_count)
    let mut index: Vec<(u32, usize, usize)> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for ctor in ctors {
        if !seen_ids.insert(ctor.id) { continue; }
        let offset = all_fields.len();
        let mut count = 0usize;
        for field in &ctor.fields {
            let ft = encode_ftype(field.ftype);
            let encoded = if let (Some(ref ff), Some(bit)) = (&field.flag_field, field.flag_bit) {
                let ffi = flag_field_index(ff) as u16;
                let b = (bit & 0x1f) as u16;
                (1u16 << 15) | (ffi << 9) | (b << 4) | ft
            } else {
                ft
            };
            all_fields.push(encoded);
            count += 1;
        }
        index.push((ctor.id, offset, count));
    }

    // sort index by ctor_id for binary search
    index.sort_by_key(|x| x.0);

    writeln!(out, "// --- field descriptor table ---").unwrap();
    writeln!(out, "static FIELD_DATA: &[u16] = &[").unwrap();
    for chunk in all_fields.chunks(20) {
        let vals: Vec<String> = chunk.iter().map(|v| format!("{:#06x}", v)).collect();
        writeln!(out, "    {},", vals.join(", ")).unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out, "").unwrap();

    // write index as (id, offset, count) sorted by id
    writeln!(out, "// (ctor_id, offset, field_count) sorted by ctor_id").unwrap();
    writeln!(out, "static CTOR_INDEX: &[(u32, u32, u16)] = &[").unwrap();
    for chunk in index.chunks(5) {
        let vals: Vec<String> = chunk.iter()
            .map(|(id, off, cnt)| format!("({:#010x}, {}, {})", id, off, cnt))
            .collect();
        writeln!(out, "    {},", vals.join(", ")).unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out, "").unwrap();
}

fn write_skip_engine(out: &mut fs::File) {
    writeln!(out, r#"
// binary search for constructor in sorted index
fn lookup_ctor(id: u32) -> Option<(usize, usize)> {{
    CTOR_INDEX.binary_search_by_key(&id, |e| e.0)
        .ok()
        .map(|i| (CTOR_INDEX[i].1 as usize, CTOR_INDEX[i].2 as usize))
}}

// skip a boxed TL object by reading its constructor and interpreting field descriptors
pub fn skip_tl(cursor: &mut Cursor<&[u8]>) -> Result<(), String> {{
    let ctor = cursor.read_u32::<LittleEndian>().map_err(|e| format!("skip_tl: read ctor: {{e}}"))?;
    skip_tl_by_id(cursor, ctor)
}}

const MAX_TL_VECTOR_ITEMS: u32 = 1_000_000;

fn check_vector_count(count: u32) -> Result<(), String> {{
    if count > MAX_TL_VECTOR_ITEMS {{
        return Err(format!("TL vector has too many items: {{count}}"));
    }}
    Ok(())
}}

pub fn skip_tl_by_id(cursor: &mut Cursor<&[u8]>, ctor: u32) -> Result<(), String> {{
    let (offset, count) = match lookup_ctor(ctor) {{
        Some(v) => v,
        None => return Err(format!("skip_tl: unknown ctor {{:#010x}}", ctor)),
    }};
    let fields = &FIELD_DATA[offset..offset + count];
    let mut flag_vals: [u32; 3] = [0; 3]; // flags, flags2, flags3
    let mut flags_seen: usize = 0;
    for &encoded in fields {{
        let ft = encoded & 0xf;
        let conditional = (encoded >> 15) & 1 != 0;
        if conditional {{
            let ffi = ((encoded >> 9) & 0x3) as usize;
            let bit = ((encoded >> 4) & 0x1f) as u32;
            if flag_vals[ffi] & (1 << bit) == 0 {{
                continue; // field not present
            }}
        }}
        match ft {{
            0 => {{ cursor.read_i32::<LittleEndian>().map_err(|_| "skip int")?; }} // int
            1 => {{ cursor.read_i64::<LittleEndian>().map_err(|_| "skip long")?; }} // long
            2 => {{ cursor.read_i64::<LittleEndian>().map_err(|_| "skip double")?; }} // double
            3 | 4 => {{ deserialize_bytes(cursor).map_err(|_| "skip str/bytes".to_string())?; }} // str/bytes
            5 => {{ skip_tl(cursor)?; }} // nested object
            6 => {{ // flags field — read and store in next slot
                let v = cursor.read_u32::<LittleEndian>().map_err(|_| "skip flags")?;
                if flags_seen < 3 {{ flag_vals[flags_seen] = v; }}
                flags_seen += 1;
            }}
            7 => {{ cursor.read_u32::<LittleEndian>().map_err(|_| "skip Bool")?; }} // boxed Bool
            8 => {{ // Vector<int>
                let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
                if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
                let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
                check_vector_count(cnt)?;
                for _ in 0..cnt {{ cursor.read_i32::<LittleEndian>().map_err(|_| "vec int")?; }}
            }}
            9 => {{ // Vector<long>
                let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
                if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
                let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
                check_vector_count(cnt)?;
                for _ in 0..cnt {{ cursor.read_i64::<LittleEndian>().map_err(|_| "vec long")?; }}
            }}
            10 | 11 => {{ // Vector<string> / Vector<bytes>
                let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
                if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
                let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
                check_vector_count(cnt)?;
                for _ in 0..cnt {{ deserialize_bytes(cursor).map_err(|_| "vec str".to_string())?; }}
            }}
            12 => {{ // Vector<Object>
                let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
                if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
                let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
                check_vector_count(cnt)?;
                for _ in 0..cnt {{ skip_tl(cursor)?; }}
            }}
            13 => {{ // int128
                let mut b = [0u8; 16]; cursor.read_exact(&mut b).map_err(|_| "skip int128")?;
            }}
            14 => {{ // int256
                let mut b = [0u8; 32]; cursor.read_exact(&mut b).map_err(|_| "skip int256")?;
            }}
            15 => {{ }} // true — no wire data
            _ => {{}}
        }}
    }}
    Ok(())
}}
"#).unwrap();
}

fn write_method_builders(out: &mut fs::File, ctors: &[Constructor]) {
    writeln!(out, "// --- method serializers ---").unwrap();
    writeln!(out, "use byteorder::WriteBytesExt;").unwrap();
    writeln!(out, "use super::tl::{{serialize_string, serialize_bytes as tl_serialize_bytes}};").unwrap();
    writeln!(out, "").unwrap();

    for ctor in ctors.iter().filter(|c| c.is_function) {
        let has_obj = ctor.fields.iter().any(|f| matches!(f.ftype, FType::Object | FType::VecObj));
        let real_fields: Vec<&Field> = ctor.fields.iter()
            .filter(|f| !matches!(f.ftype, FType::True | FType::Flags))
            .collect();
        let has_flags = ctor.fields.iter().any(|f| matches!(f.ftype, FType::Flags));
        let fn_name = format!("build_{}", ctor.name.replace('.', "_"));

        if real_fields.is_empty() && !has_flags {
            writeln!(out, "pub fn {}() -> Vec<u8> {{", fn_name).unwrap();
            writeln!(out, "    let mut buf = Vec::new();").unwrap();
            writeln!(out, "    buf.write_u32::<LittleEndian>({:#010x}).unwrap();", ctor.id).unwrap();
            writeln!(out, "    buf").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
            continue;
        }

        if !has_obj {
            if !has_flags && real_fields.iter().all(|f| f.flag_field.is_none()) {
                write_simple_builder(out, ctor, &real_fields, &fn_name);
            } else if has_flags {
                write_flagged_builder(out, ctor, &fn_name);
            }
        } else {
            // methods with Object fields — accept &[u8] for pre-serialized TL
            write_raw_builder(out, ctor, &fn_name);
        }
    }

    write_vector_helpers(out);
    write_field_readers(out);
    write_rpc_helpers(out);
    write_peer_helpers(out);
    write_wrap_helpers(out);
}

fn sanitize_name(name: &str) -> String {
    match name {
        "self" => "self_".to_string(),
        "Self" => "self_type".to_string(),
        "type" | "loop" | "move" | "ref" | "match" | "mod" | "use" |
        "fn" | "let" | "mut" | "pub" | "return" | "where" | "async" | "await" |
        "in" | "for" | "if" | "else" | "while" | "break" | "continue" | "struct" |
        "enum" | "trait" | "impl" | "static" | "const" | "super" | "crate" |
        "box" | "yield" | "dyn" | "abstract" | "final" | "override" | "macro" =>
            format!("r#{}", name),
        _ => name.to_string(),
    }
}

fn rust_type_for(ft: FType) -> &'static str {
    match ft {
        FType::Int => "i32",
        FType::Long => "i64",
        FType::Double => "f64",
        FType::Str => "&str",
        FType::Bytes => "&[u8]",
        FType::Bool => "bool",
        FType::VecInt => "&[i32]",
        FType::VecLong => "&[i64]",
        FType::VecStr => "&[&str]",
        FType::VecBytes => "&[&[u8]]",
        _ => "UNSUPPORTED",
    }
}

fn write_simple_builder(out: &mut fs::File, ctor: &Constructor, fields: &[&Field], fn_name: &str) {
    // build function signature
    let params: Vec<String> = fields.iter().map(|f| {
        format!("{}: {}", sanitize_name(&f.name), rust_type_for(f.ftype))
    }).collect();
    writeln!(out, "pub fn {}({}) -> Vec<u8> {{", fn_name, params.join(", ")).unwrap();
    writeln!(out, "    let mut buf = Vec::new();").unwrap();
    writeln!(out, "    buf.write_u32::<LittleEndian>({:#010x}).unwrap();", ctor.id).unwrap();

    for field in fields {
        write_serialize_value(out, field.ftype, &sanitize_name(&field.name), "    ");
    }

    writeln!(out, "    buf").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();
}

fn write_flagged_builder(out: &mut fs::File, ctor: &Constructor, fn_name: &str) {
    // for flagged methods, generate a builder with Option<T> for conditional fields
    // and bool for True fields
    let mut params: Vec<String> = Vec::new();
    for field in &ctor.fields {
        match field.ftype {
            FType::Flags => continue,
            FType::True => {
                // true fields become bool parameters
                params.push(format!("{}: bool", sanitize_name(&field.name)));
                continue;
            }
            _ => {}
        }
        let base_type = rust_type_for(field.ftype);
        if base_type == "UNSUPPORTED" { return; } // skip complex methods
        let sname = sanitize_name(&field.name);
        if field.flag_field.is_some() {
            params.push(format!("{}: Option<{}>", sname, base_type));
        } else {
            params.push(format!("{}: {}", sname, base_type));
        }
    }

    writeln!(out, "pub fn {}({}) -> Vec<u8> {{", fn_name, params.join(", ")).unwrap();
    writeln!(out, "    let mut buf = Vec::new();").unwrap();
    writeln!(out, "    buf.write_u32::<LittleEndian>({:#010x}).unwrap();", ctor.id).unwrap();

    // compute flags value
    let flag_fields: Vec<&Field> = ctor.fields.iter()
        .filter(|f| matches!(f.ftype, FType::Flags))
        .collect();

    for ff in &flag_fields {
        let mut flag_expr_parts: Vec<String> = Vec::new();
        for field in &ctor.fields {
            if field.flag_field.as_deref() == Some(&ff.name) {
                let bit = field.flag_bit.unwrap();
                let sname = sanitize_name(&field.name);
                if matches!(field.ftype, FType::True) {
                    flag_expr_parts.push(format!("(if {} {{ 1u32 << {} }} else {{ 0 }})", sname, bit));
                } else {
                    flag_expr_parts.push(format!("(if {}.is_some() {{ 1u32 << {} }} else {{ 0 }})", sname, bit));
                }
            }
        }
        if flag_expr_parts.is_empty() {
            writeln!(out, "    buf.write_u32::<LittleEndian>(0).unwrap();").unwrap();
        } else {
            writeln!(out, "    let {}_val: u32 = {};", ff.name, flag_expr_parts.join(" | ")).unwrap();
            writeln!(out, "    buf.write_u32::<LittleEndian>({}_val).unwrap();", ff.name).unwrap();
        }
    }

    // serialize fields
    for field in &ctor.fields {
        match field.ftype {
            FType::Flags | FType::True => continue,
            _ => {}
        }
        let sname = sanitize_name(&field.name);
        if field.flag_field.is_some() {
            writeln!(out, "    if let Some(v) = {} {{", sname).unwrap();
            write_serialize_value(out, field.ftype, "v", "        ");
            writeln!(out, "    }}").unwrap();
        } else {
            write_serialize_value(out, field.ftype, &sname, "    ");
        }
    }

    writeln!(out, "    buf").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();
}

fn write_serialize_field(out: &mut fs::File, field: &Field, indent: &str) {
    write_serialize_value(out, field.ftype, &field.name, indent);
}

fn write_serialize_value(out: &mut fs::File, ftype: FType, var: &str, indent: &str) {
    match ftype {
        FType::Int => writeln!(out, "{}buf.write_i32::<LittleEndian>({}).unwrap();", indent, var).unwrap(),
        FType::Long => writeln!(out, "{}buf.write_i64::<LittleEndian>({}).unwrap();", indent, var).unwrap(),
        FType::Double => writeln!(out, "{}buf.write_i64::<LittleEndian>({}.to_bits() as i64).unwrap();", indent, var).unwrap(),
        FType::Str => writeln!(out, "{}buf.extend(serialize_string({}));", indent, var).unwrap(),
        FType::Bytes => writeln!(out, "{}buf.extend(tl_serialize_bytes({}));", indent, var).unwrap(),
        FType::Bool => writeln!(out, "{}buf.write_u32::<LittleEndian>(if {} {{ 0x997275b5 }} else {{ 0xbc799737 }}).unwrap();", indent, var).unwrap(),
        FType::VecInt => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for &item in {} {{ buf.write_i32::<LittleEndian>(item).unwrap(); }}", indent, var).unwrap();
        }
        FType::VecLong => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for &item in {} {{ buf.write_i64::<LittleEndian>(item).unwrap(); }}", indent, var).unwrap();
        }
        FType::VecStr => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for item in {} {{ buf.extend(serialize_string(item)); }}", indent, var).unwrap();
        }
        FType::VecBytes => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for item in {} {{ buf.extend(tl_serialize_bytes(item)); }}", indent, var).unwrap();
        }
        _ => {}
    }
}

fn write_vector_helpers(out: &mut fs::File) {
    writeln!(out, r#"
// skip a Vector<T> where T is a boxed TL object
pub fn skip_vector(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "skip_vector: ctor")?;
    if vc != 0x1cb5c415 {{ return Err(format!("skip_vector: expected vector, got {{:#x}}", vc)); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "skip_vector: count")?;
    check_vector_count(cnt)?;
    for _ in 0..cnt {{ skip_tl(cursor)?; }}
    Ok(cnt)
}}

// skip a Vector<int>
pub fn skip_vector_int(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    for _ in 0..cnt {{ cursor.read_i32::<LittleEndian>().map_err(|_| "vec int")?; }}
    Ok(cnt)
}}

// skip a Vector<long>
pub fn skip_vector_long(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    for _ in 0..cnt {{ cursor.read_i64::<LittleEndian>().map_err(|_| "vec long")?; }}
    Ok(cnt)
}}

// skip a Vector<string>
pub fn skip_vector_string(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    for _ in 0..cnt {{ deserialize_bytes(cursor).map_err(|_| "vec str".to_string())?; }}
    Ok(cnt)
}}

// read a Vector<long> and return the values
pub fn read_vector_long(cursor: &mut Cursor<&[u8]>) -> Result<Vec<i64>, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    let mut v = Vec::with_capacity(cnt as usize);
    for _ in 0..cnt {{ v.push(cursor.read_i64::<LittleEndian>().map_err(|_| "vec long")?); }}
    Ok(v)
}}

// read a Vector<int> and return the values
pub fn read_vector_int(cursor: &mut Cursor<&[u8]>) -> Result<Vec<i32>, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    let mut v = Vec::with_capacity(cnt as usize);
    for _ in 0..cnt {{ v.push(cursor.read_i32::<LittleEndian>().map_err(|_| "vec int")?); }}
    Ok(v)
}}

// read a Vector<string> and return the values
pub fn read_vector_string(cursor: &mut Cursor<&[u8]>) -> Result<Vec<String>, String> {{
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err("not a vector".into()); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    let mut v = Vec::with_capacity(cnt as usize);
    for _ in 0..cnt {{ v.push(deserialize_string(cursor)?); }}
    Ok(v)
}}
"#).unwrap();
}

fn write_field_readers(out: &mut fs::File) {
    writeln!(out, r#"
// check if a constructor is known in the schema
pub fn is_known_ctor(id: u32) -> bool {{
    lookup_ctor(id).is_some()
}}

// get field count for a constructor
pub fn ctor_field_count(id: u32) -> Option<usize> {{
    lookup_ctor(id).map(|(_, count)| count)
}}

// --- typed Object field accessor ---
// deserialize raw TL bytes (Vec<u8>) into a concrete type
// usage: let photo = deserialize_tl_obj::<TlUserProfilePhoto>(&user.photo)?;
pub fn deserialize_tl_obj<T: TlDeserialize>(raw: &[u8]) -> Result<T, String> {{
    let mut cursor = Cursor::new(raw);
    T::tl_deserialize(&mut cursor)
}}

// deserialize a vector of raw TL objects
pub fn deserialize_tl_vec<T: TlDeserialize>(raw_vec: &[Vec<u8>]) -> Result<Vec<T>, String> {{
    raw_vec.iter().map(|raw| deserialize_tl_obj::<T>(raw)).collect()
}}

// trait for types that can be deserialized from a TL cursor
pub trait TlDeserialize: Sized {{
    fn tl_deserialize(cursor: &mut Cursor<&[u8]>) -> Result<Self, String>;
}}
"#).unwrap();

    // now emit TlDeserialize impls for all generated structs and enums
    // (we'll do this in a separate pass after all types are generated)

    writeln!(out, r#"
// serialize a bare TL constructor (just the ID, no fields)
pub fn serialize_bare_ctor(id: u32) -> Vec<u8> {{
    id.to_le_bytes().to_vec()
}}

// serialize inputPeerUser(user_id, access_hash)
pub fn serialize_input_peer_user(user_id: i64, access_hash: i64) -> Vec<u8> {{
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&INPUT_PEER_USER.to_le_bytes());
    buf.extend_from_slice(&user_id.to_le_bytes());
    buf.extend_from_slice(&access_hash.to_le_bytes());
    buf
}}

// serialize inputPeerChannel(channel_id, access_hash)
pub fn serialize_input_peer_channel(channel_id: i64, access_hash: i64) -> Vec<u8> {{
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&INPUT_PEER_CHANNEL.to_le_bytes());
    buf.extend_from_slice(&channel_id.to_le_bytes());
    buf.extend_from_slice(&access_hash.to_le_bytes());
    buf
}}

// serialize inputPeerSelf
pub fn serialize_input_peer_self() -> Vec<u8> {{
    INPUT_PEER_SELF.to_le_bytes().to_vec()
}}

// serialize inputPeerChat(chat_id)
pub fn serialize_input_peer_chat(chat_id: i64) -> Vec<u8> {{
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&INPUT_PEER_CHAT.to_le_bytes());
    buf.extend_from_slice(&chat_id.to_le_bytes());
    buf
}}

// serialize inputUser(user_id, access_hash)
pub fn serialize_input_user(user_id: i64, access_hash: i64) -> Vec<u8> {{
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&INPUT_USER.to_le_bytes());
    buf.extend_from_slice(&user_id.to_le_bytes());
    buf.extend_from_slice(&access_hash.to_le_bytes());
    buf
}}

// serialize inputUserSelf
pub fn serialize_input_user_self() -> Vec<u8> {{
    INPUT_USER_SELF.to_le_bytes().to_vec()
}}

// serialize inputChannel(channel_id, access_hash)
pub fn serialize_input_channel(channel_id: i64, access_hash: i64) -> Vec<u8> {{
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&INPUT_CHANNEL.to_le_bytes());
    buf.extend_from_slice(&channel_id.to_le_bytes());
    buf.extend_from_slice(&access_hash.to_le_bytes());
    buf
}}
"#).unwrap();
}

fn write_raw_builder(out: &mut fs::File, ctor: &Constructor, fn_name: &str) {
    // for methods with Object/VecObj fields, accept &[u8] for pre-serialized TL
    let has_flags = ctor.fields.iter().any(|f| matches!(f.ftype, FType::Flags));
    let mut params: Vec<String> = Vec::new();

    for field in &ctor.fields {
        match field.ftype {
            FType::Flags => continue,
            FType::True => {
                params.push(format!("{}: bool", sanitize_name(&field.name)));
                continue;
            }
            _ => {}
        }
        let sname = sanitize_name(&field.name);
        let ty = match field.ftype {
            FType::Object => "&[u8]",
            FType::VecObj => "&[&[u8]]",
            FType::Int => "i32",
            FType::Long => "i64",
            FType::Double => "f64",
            FType::Str => "&str",
            FType::Bytes => "&[u8]",
            FType::Bool => "bool",
            FType::VecInt => "&[i32]",
            FType::VecLong => "&[i64]",
            FType::VecStr => "&[&str]",
            FType::VecBytes => "&[&[u8]]",
            _ => "UNSUPPORTED",
        };
        if ty == "UNSUPPORTED" { return; }
        if field.flag_field.is_some() {
            params.push(format!("{}: Option<{}>", sname, ty));
        } else {
            params.push(format!("{}: {}", sname, ty));
        }
    }

    writeln!(out, "pub fn {}({}) -> Vec<u8> {{", fn_name, params.join(", ")).unwrap();
    writeln!(out, "    let mut buf = Vec::new();").unwrap();
    writeln!(out, "    buf.write_u32::<LittleEndian>({:#010x}).unwrap();", ctor.id).unwrap();

    if has_flags {
        let flag_fields: Vec<&Field> = ctor.fields.iter()
            .filter(|f| matches!(f.ftype, FType::Flags))
            .collect();
        for ff in &flag_fields {
            let mut parts: Vec<String> = Vec::new();
            for field in &ctor.fields {
                if field.flag_field.as_deref() == Some(&ff.name) {
                    let bit = field.flag_bit.unwrap();
                    let sname = sanitize_name(&field.name);
                    if matches!(field.ftype, FType::True) {
                        parts.push(format!("(if {} {{ 1u32 << {} }} else {{ 0 }})", sname, bit));
                    } else {
                        parts.push(format!("(if {}.is_some() {{ 1u32 << {} }} else {{ 0 }})", sname, bit));
                    }
                }
            }
            if parts.is_empty() {
                writeln!(out, "    buf.write_u32::<LittleEndian>(0).unwrap();").unwrap();
            } else {
                writeln!(out, "    let {}_val: u32 = {};", ff.name, parts.join(" | ")).unwrap();
                writeln!(out, "    buf.write_u32::<LittleEndian>({}_val).unwrap();", ff.name).unwrap();
            }
        }
    }

    for field in &ctor.fields {
        match field.ftype {
            FType::Flags | FType::True => continue,
            _ => {}
        }
        let sname = sanitize_name(&field.name);
        if field.flag_field.is_some() {
            writeln!(out, "    if let Some(v) = {} {{", sname).unwrap();
            write_raw_serialize(out, field.ftype, "v", "        ");
            writeln!(out, "    }}").unwrap();
        } else {
            write_raw_serialize(out, field.ftype, &sname, "    ");
        }
    }

    writeln!(out, "    buf").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();
}

fn write_raw_serialize(out: &mut fs::File, ftype: FType, var: &str, indent: &str) {
    match ftype {
        FType::Int => writeln!(out, "{}buf.write_i32::<LittleEndian>({}).unwrap();", indent, var).unwrap(),
        FType::Long => writeln!(out, "{}buf.write_i64::<LittleEndian>({}).unwrap();", indent, var).unwrap(),
        FType::Double => writeln!(out, "{}buf.write_i64::<LittleEndian>({}.to_bits() as i64).unwrap();", indent, var).unwrap(),
        FType::Str => writeln!(out, "{}buf.extend(serialize_string({}));", indent, var).unwrap(),
        FType::Bytes => writeln!(out, "{}buf.extend(tl_serialize_bytes({}));", indent, var).unwrap(),
        FType::Bool => writeln!(out, "{}buf.write_u32::<LittleEndian>(if {} {{ 0x997275b5 }} else {{ 0xbc799737 }}).unwrap();", indent, var).unwrap(),
        FType::Object => writeln!(out, "{}buf.extend_from_slice({});", indent, var).unwrap(),
        FType::VecObj => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for item in {} {{ buf.extend_from_slice(item); }}", indent, var).unwrap();
        }
        FType::VecInt => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for &item in {} {{ buf.write_i32::<LittleEndian>(item).unwrap(); }}", indent, var).unwrap();
        }
        FType::VecLong => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for &item in {} {{ buf.write_i64::<LittleEndian>(item).unwrap(); }}", indent, var).unwrap();
        }
        FType::VecStr => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for item in {} {{ buf.extend(serialize_string(item)); }}", indent, var).unwrap();
        }
        FType::VecBytes => {
            writeln!(out, "{}buf.write_u32::<LittleEndian>(0x1cb5c415).unwrap();", indent).unwrap();
            writeln!(out, "{}buf.write_u32::<LittleEndian>({}.len() as u32).unwrap();", indent, var).unwrap();
            writeln!(out, "{}for item in {} {{ buf.extend(tl_serialize_bytes(item)); }}", indent, var).unwrap();
        }
        _ => {}
    }
}

fn write_rpc_helpers(out: &mut fs::File) {
    writeln!(out, r#"
// --- RPC response helpers ---

// structured RPC error
#[derive(Debug, Clone)]
pub struct RpcError {{
    pub code: i32,
    pub message: String,
}}

impl RpcError {{
    pub fn is_flood(&self) -> bool {{ self.message.starts_with("FLOOD_WAIT_") || self.message.starts_with("FLOOD_PREMIUM_WAIT_") }}
    pub fn flood_seconds(&self) -> Option<u64> {{
        if let Some(s) = self.message.strip_prefix("FLOOD_WAIT_") {{
            s.parse().ok()
        }} else if let Some(s) = self.message.strip_prefix("FLOOD_PREMIUM_WAIT_") {{
            s.parse().ok()
        }} else {{ None }}
    }}
    pub fn is_auth_key_unregistered(&self) -> bool {{ self.message == "AUTH_KEY_UNREGISTERED" }}
    pub fn is_session_revoked(&self) -> bool {{ self.message == "SESSION_REVOKED" }}
    pub fn is_user_deactivated(&self) -> bool {{ self.message.starts_with("USER_DEACTIVATED") }}
    pub fn is_phone_migrate(&self) -> bool {{ self.message.starts_with("PHONE_MIGRATE_") }}
    pub fn migrate_dc(&self) -> Option<i32> {{
        for prefix in &["PHONE_MIGRATE_", "USER_MIGRATE_", "NETWORK_MIGRATE_", "FILE_MIGRATE_"] {{
            if let Some(s) = self.message.strip_prefix(prefix) {{
                return s.parse().ok();
            }}
        }}
        None
    }}
}}

impl std::fmt::Display for RpcError {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        write!(f, "RPC {{}}: {{}}", self.code, self.message)
    }}
}}

// result type for RPC calls — either success payload or structured error
pub type RpcResult<T> = Result<T, RpcError>;

// unwrap rpc_result + gzip, returning the inner payload or structured error
pub fn unwrap_rpc(data: &[u8]) -> Result<Vec<u8>, String> {{
    use super::tl::decompress_gzip;
    if data.len() < 4 {{ return Err("rpc response too short".into()); }}
    let mut cursor = Cursor::new(data);
    let ctor = cursor.read_u32::<LittleEndian>().map_err(|_| "read ctor")?;
    match ctor {{
        0xf35c6d01 => {{ // rpc_result
            let _req_id = cursor.read_u64::<LittleEndian>().map_err(|_| "read req_id")?;
            let pos = cursor.position() as usize;
            let inner = &data[pos..];
            if inner.len() >= 4 {{
                let inner_ctor = u32::from_le_bytes([inner[0], inner[1], inner[2], inner[3]]);
                if inner_ctor == 0x3072cfa1 {{
                    let mut ic = Cursor::new(inner);
                    ic.set_position(4);
                    let compressed = deserialize_bytes(&mut ic).map_err(|_| "gzip bytes".to_string())?;
                    return decompress_gzip(&compressed);
                }}
                if inner_ctor == 0x2144ca19 {{
                    let mut ic = Cursor::new(inner);
                    ic.set_position(4);
                    let code = ic.read_i32::<LittleEndian>().map_err(|_| "err code")?;
                    let msg = deserialize_string(&mut ic)?;
                    return Err(format!("RPC {{}}: {{}}", code, msg));
                }}
            }}
            Ok(inner.to_vec())
        }}
        0x73f1f8dc => {{ // msg_container
            let count = cursor.read_u32::<LittleEndian>().map_err(|_| "container count")?;
            for _ in 0..count {{
                let _msg_id = cursor.read_u64::<LittleEndian>().map_err(|_| "msg_id")?;
                let _seq = cursor.read_u32::<LittleEndian>().map_err(|_| "seq")?;
                let body_len = cursor.read_u32::<LittleEndian>().map_err(|_| "body_len")? as usize;
                let pos = cursor.position() as usize;
                let body = &data[pos..pos + body_len.min(data.len() - pos)];
                if body.len() >= 4 {{
                    let ic = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
                    if ic == 0xf35c6d01 {{ return unwrap_rpc(body); }}
                }}
                cursor.set_position((pos + body_len) as u64);
            }}
            Err("no rpc_result in container".into())
        }}
        0x3072cfa1 => {{ // gzip_packed
            let compressed = deserialize_bytes(&mut cursor).map_err(|_| "gzip bytes".to_string())?;
            decompress_gzip(&compressed)
        }}
        0x2144ca19 => {{ // rpc_error
            let code = cursor.read_i32::<LittleEndian>().map_err(|_| "err code")?;
            let msg = deserialize_string(&mut cursor)?;
            Err(format!("RPC {{}}: {{}}", code, msg))
        }}
        _ => Ok(data.to_vec())
    }}
}}

// structured unwrap — returns RpcResult instead of String error
pub fn unwrap_rpc_typed(data: &[u8]) -> RpcResult<Vec<u8>> {{
    match unwrap_rpc(data) {{
        Ok(v) => Ok(v),
        Err(e) => {{
            // parse "RPC CODE: MESSAGE" format
            if let Some(rest) = e.strip_prefix("RPC ") {{
                if let Some(colon) = rest.find(": ") {{
                    let code = rest[..colon].parse::<i32>().unwrap_or(0);
                    let message = rest[colon + 2..].to_string();
                    return Err(RpcError {{ code, message }});
                }}
            }}
            Err(RpcError {{ code: 0, message: e }})
        }}
    }}
}}

// parse Bool response (boolTrue/boolFalse)
pub fn parse_bool(data: &[u8]) -> Result<bool, String> {{
    let inner = unwrap_rpc(data)?;
    if inner.len() < 4 {{ return Err("bool response too short".into()); }}
    let ctor = u32::from_le_bytes([inner[0], inner[1], inner[2], inner[3]]);
    Ok(ctor == 0x997275b5) // boolTrue
}}

// parse Bool response with structured error
pub fn parse_bool_typed(data: &[u8]) -> RpcResult<bool> {{
    let inner = unwrap_rpc_typed(data)?;
    if inner.len() < 4 {{ return Err(RpcError {{ code: 0, message: "bool too short".into() }}); }}
    let ctor = u32::from_le_bytes([inner[0], inner[1], inner[2], inner[3]]);
    Ok(ctor == 0x997275b5)
}}

// parse Vector<int> response
pub fn parse_vector_int_response(data: &[u8]) -> Result<Vec<i32>, String> {{
    let inner = unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());
    read_vector_int(&mut cursor)
}}

// parse Vector<long> response
pub fn parse_vector_long_response(data: &[u8]) -> Result<Vec<i64>, String> {{
    let inner = unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());
    read_vector_long(&mut cursor)
}}

// parse Vector<T> response where T implements TlDeserialize
pub fn parse_vector_response<T: TlDeserialize>(data: &[u8]) -> Result<Vec<T>, String> {{
    let inner = unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());
    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| "vec ctor")?;
    if vc != 0x1cb5c415 {{ return Err(format!("expected vector, got {{:#x}}", vc)); }}
    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| "vec count")?;
    check_vector_count(cnt)?;
    let mut v = Vec::with_capacity(cnt as usize);
    for _ in 0..cnt {{
        v.push(T::tl_deserialize(&mut cursor)?);
    }}
    Ok(v)
}}

// --- flood wait retry policy ---

#[derive(Debug, Clone)]
pub struct FloodPolicy {{
    pub wait: bool,          // whether to wait at all
    pub max_wait_secs: u64,  // maximum seconds to wait (0 = unlimited)
    pub max_retries: u32,    // maximum retry attempts (0 = unlimited)
}}

impl Default for FloodPolicy {{
    fn default() -> Self {{
        Self {{ wait: true, max_wait_secs: 60, max_retries: 3 }}
    }}
}}

impl FloodPolicy {{
    pub fn no_wait() -> Self {{ Self {{ wait: false, max_wait_secs: 0, max_retries: 0 }} }}
    pub fn wait_up_to(secs: u64) -> Self {{ Self {{ wait: true, max_wait_secs: secs, max_retries: 5 }} }}
    pub fn unlimited() -> Self {{ Self {{ wait: true, max_wait_secs: 0, max_retries: 0 }} }}
}}

// action to take after checking flood
pub enum FloodAction {{
    Proceed,                // no flood, proceed normally
    Wait(u64),             // wait N seconds then retry
    Abort(RpcError),       // flood exceeds policy, abort
}}

impl RpcError {{
    // check flood against policy and return action
    pub fn check_flood(&self, policy: &FloodPolicy) -> FloodAction {{
        if !self.is_flood() {{
            return FloodAction::Abort(self.clone());
        }}
        if !policy.wait {{
            return FloodAction::Abort(self.clone());
        }}
        let secs = self.flood_seconds().unwrap_or(30);
        if policy.max_wait_secs > 0 && secs > policy.max_wait_secs {{
            return FloodAction::Abort(self.clone());
        }}
        FloodAction::Wait(secs)
    }}
}}
"#).unwrap();
}

fn write_peer_helpers(out: &mut fs::File) {
    writeln!(out, r#"
// --- Peer deserialization ---

#[derive(Debug, Clone, PartialEq)]
pub enum Peer {{
    User(i64),
    Chat(i64),
    Channel(i64),
}}

// read a Peer from cursor (peerUser/peerChat/peerChannel)
pub fn read_peer(cursor: &mut Cursor<&[u8]>) -> Result<Peer, String> {{
    let ctor = cursor.read_u32::<LittleEndian>().map_err(|_| "peer ctor")?;
    let id = cursor.read_i64::<LittleEndian>().map_err(|_| "peer id")?;
    match ctor {{
        0x59511722 => Ok(Peer::User(id)),
        0x36c6019a => Ok(Peer::Chat(id)),
        0xa2a5371e => Ok(Peer::Channel(id)),
        _ => Err(format!("unknown peer ctor {{:#x}}", ctor)),
    }}
}}

// skip a Peer (always 12 bytes: ctor + id)
pub fn skip_peer(cursor: &mut Cursor<&[u8]>) -> Result<(), String> {{
    let _ = cursor.read_u32::<LittleEndian>().map_err(|_| "peer ctor")?;
    let _ = cursor.read_i64::<LittleEndian>().map_err(|_| "peer id")?;
    Ok(())
}}

impl Peer {{
    pub fn id(&self) -> i64 {{
        match self {{
            Peer::User(id) | Peer::Chat(id) | Peer::Channel(id) => *id,
        }}
    }}

    pub fn is_user(&self) -> bool {{ matches!(self, Peer::User(_)) }}
    pub fn is_chat(&self) -> bool {{ matches!(self, Peer::Chat(_)) }}
    pub fn is_channel(&self) -> bool {{ matches!(self, Peer::Channel(_)) }}
}}
"#).unwrap();
}

fn write_wrap_helpers(out: &mut fs::File) {
    writeln!(out, r#"
// --- invokeWithLayer + initConnection wrapper ---

pub const CURRENT_LAYER: i32 = 228;

// wrap a request in invokeWithLayer(layer, initConnection(..., query))
pub fn wrap_invoke_with_layer(
    inner: &[u8],
    api_id: i32,
    device: &str,
    system: &str,
    app_version: &str,
    system_lang: &str,
    lang: &str,
) -> Vec<u8> {{
    let mut init = Vec::new();
    init.write_u32::<LittleEndian>(INIT_CONNECTION).unwrap();
    init.write_u32::<LittleEndian>(0).unwrap(); // flags
    init.write_i32::<LittleEndian>(api_id).unwrap();
    init.extend(serialize_string(device));
    init.extend(serialize_string(system));
    init.extend(serialize_string(app_version));
    init.extend(serialize_string(system_lang));
    init.extend(serialize_string("")); // lang_pack
    init.extend(serialize_string(lang));
    init.extend_from_slice(inner);

    let mut req = Vec::new();
    req.write_u32::<LittleEndian>(INVOKE_WITH_LAYER).unwrap();
    req.write_i32::<LittleEndian>(CURRENT_LAYER).unwrap();
    req.extend(&init);
    req
}}

// build users.getUsers([inputUserSelf]) — common first request
pub fn build_get_me(
    api_id: i32,
    device: &str,
    system: &str,
    app_version: &str,
    system_lang: &str,
    lang: &str,
) -> Vec<u8> {{
    let mut inner = Vec::new();
    inner.write_u32::<LittleEndian>(USERS_GET_USERS).unwrap();
    inner.write_u32::<LittleEndian>(0x1cb5c415).unwrap(); // vector
    inner.write_u32::<LittleEndian>(1).unwrap(); // count
    inner.write_u32::<LittleEndian>(INPUT_USER_SELF).unwrap();
    wrap_invoke_with_layer(&inner, api_id, device, system, app_version, system_lang, lang)
}}
"#).unwrap();
}

fn write_type_serializers(out: &mut fs::File, ctors: &[Constructor]) {
    writeln!(out, "// --- type constructor serializers ---").unwrap();
    writeln!(out, "").unwrap();

    for ctor in ctors.iter().filter(|c| !c.is_function) {
        let real_fields: Vec<&Field> = ctor.fields.iter()
            .filter(|f| !matches!(f.ftype, FType::True | FType::Flags))
            .collect();
        if real_fields.is_empty() && !ctor.fields.iter().any(|f| matches!(f.ftype, FType::Flags)) {
            continue;
        }

        let fn_name = format!("serialize_{}", ctor.name.replace('.', "_"));
        let has_flags = ctor.fields.iter().any(|f| matches!(f.ftype, FType::Flags));

        // build params — Object/VecObj become &[u8] / &[&[u8]]
        let mut params: Vec<String> = Vec::new();
        let mut has_unsupported = false;
        for field in &ctor.fields {
            match field.ftype {
                FType::Flags => continue,
                FType::True => {
                    params.push(format!("{}: bool", sanitize_name(&field.name)));
                    continue;
                }
                _ => {}
            }
            let ty = match field.ftype {
                FType::Object => "&[u8]",
                FType::VecObj => "&[&[u8]]",
                _ => rust_type_for(field.ftype),
            };
            if ty == "UNSUPPORTED" { has_unsupported = true; break; }
            let sname = sanitize_name(&field.name);
            if field.flag_field.is_some() {
                params.push(format!("{}: Option<{}>", sname, ty));
            } else {
                params.push(format!("{}: {}", sname, ty));
            }
        }
        if has_unsupported { continue; }

        writeln!(out, "pub fn {}({}) -> Vec<u8> {{", fn_name, params.join(", ")).unwrap();
        writeln!(out, "    let mut buf = Vec::new();").unwrap();
        writeln!(out, "    buf.write_u32::<LittleEndian>({:#010x}).unwrap();", ctor.id).unwrap();

        if has_flags {
            let flag_fields: Vec<&Field> = ctor.fields.iter()
                .filter(|f| matches!(f.ftype, FType::Flags))
                .collect();
            for ff in &flag_fields {
                let mut parts: Vec<String> = Vec::new();
                for field in &ctor.fields {
                    if field.flag_field.as_deref() == Some(&ff.name) {
                        let bit = field.flag_bit.unwrap();
                        let sname = sanitize_name(&field.name);
                        if matches!(field.ftype, FType::True) {
                            parts.push(format!("(if {} {{ 1u32 << {} }} else {{ 0 }})", sname, bit));
                        } else {
                            parts.push(format!("(if {}.is_some() {{ 1u32 << {} }} else {{ 0 }})", sname, bit));
                        }
                    }
                }
                if parts.is_empty() {
                    writeln!(out, "    buf.write_u32::<LittleEndian>(0).unwrap();").unwrap();
                } else {
                    writeln!(out, "    let {}_val: u32 = {};", ff.name, parts.join(" | ")).unwrap();
                    writeln!(out, "    buf.write_u32::<LittleEndian>({}_val).unwrap();", ff.name).unwrap();
                }
            }
        }

        for field in &ctor.fields {
            match field.ftype {
                FType::Flags | FType::True => continue,
                _ => {}
            }
            let sname = sanitize_name(&field.name);
            if field.flag_field.is_some() {
                writeln!(out, "    if let Some(v) = {} {{", sname).unwrap();
                write_raw_serialize(out, field.ftype, "v", "        ");
                writeln!(out, "    }}").unwrap();
            } else {
                write_raw_serialize(out, field.ftype, &sname, "    ");
            }
        }
        writeln!(out, "    buf").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out, "").unwrap();
    }
}

fn write_type_deserializers(out: &mut fs::File, ctors: &[Constructor]) {
    // group non-function constructors by result_type
    let mut by_type: HashMap<String, Vec<&Constructor>> = HashMap::new();
    for ctor in ctors.iter().filter(|c| !c.is_function) {
        // skip types with spaces or generic params in name
        if ctor.result_type.contains(' ') || ctor.result_type.contains('<') {
            continue;
        }
        by_type.entry(ctor.result_type.clone()).or_default().push(ctor);
    }

    writeln!(out, "// --- type deserializers ---").unwrap();
    writeln!(out, "").unwrap();

    // generate structs + deserialize for types with a single constructor
    // that have only primitive fields (no nested Object requiring recursive deser)
    for (type_name, type_ctors) in &by_type {
        if type_ctors.len() == 1 {
            let ctor = type_ctors[0];
            write_single_ctor_struct(out, ctor, type_name);
        } else {
            write_enum_type(out, type_ctors, type_name);
        }
    }
}

fn to_struct_name(type_name: &str) -> String {
    // convert "messages.Dialogs" -> "TlMessagesDialogs", "User" -> "TlUser"
    let parts: Vec<&str> = type_name.split('.').collect();
    let mut name = String::from("Tl");
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(chars);
        }
    }
    name
}

fn to_snake(name: &str) -> String {
    let mut result = String::new();
    let mut prev_upper = false;
    for (i, ch) in name.chars().enumerate() {
        if ch == '.' { result.push('_'); prev_upper = false; continue; }
        if ch.is_uppercase() {
            if i > 0 && !prev_upper { result.push('_'); }
            result.push(ch.to_ascii_lowercase());
            prev_upper = true;
        } else {
            result.push(ch);
            prev_upper = false;
        }
    }
    result
}

fn rust_owned_type(ft: FType) -> &'static str {
    match ft {
        FType::Int => "i32",
        FType::Long => "i64",
        FType::Double => "f64",
        FType::Str => "String",
        FType::Bytes => "Vec<u8>",
        FType::Bool => "bool",
        FType::True => "bool",
        FType::Flags => "u32",
        FType::VecInt => "Vec<i32>",
        FType::VecLong => "Vec<i64>",
        FType::VecStr => "Vec<String>",
        FType::VecBytes => "Vec<Vec<u8>>",
        FType::Object => "Vec<u8>",
        FType::VecObj => "Vec<Vec<u8>>",
        FType::Int128 => "[u8; 16]",
        FType::Int256 => "[u8; 32]",
    }
}

fn write_single_ctor_struct(out: &mut fs::File, ctor: &Constructor, type_name: &str) {
    let struct_name = to_struct_name(type_name);
    let real_fields: Vec<&Field> = ctor.fields.iter()
        .filter(|f| !matches!(f.ftype, FType::Flags))
        .collect();

    // pre-check: all field types must be representable
    for field in &real_fields {
        if matches!(field.ftype, FType::True) { continue; }
        let ty = rust_owned_type(field.ftype);
        if ty == "SKIP" { return; }
    }

    writeln!(out, "#[derive(Debug, Clone, Default)]").unwrap();
    writeln!(out, "pub struct {} {{", struct_name).unwrap();
    for field in &real_fields {
        let sname = sanitize_name(&field.name);
        let ty = rust_owned_type(field.ftype);
        if field.flag_field.is_some() && !matches!(field.ftype, FType::True) {
            writeln!(out, "    pub {}: Option<{}>,", sname, ty).unwrap();
        } else {
            writeln!(out, "    pub {}: {},", sname, ty).unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();

    writeln!(out, "impl {} {{", struct_name).unwrap();
    writeln!(out, "    pub fn deserialize(cursor: &mut Cursor<&[u8]>) -> Result<Self, String> {{").unwrap();
    writeln!(out, "        let mut obj = Self::default();").unwrap();

    let flag_fields: Vec<&Field> = ctor.fields.iter()
        .filter(|f| matches!(f.ftype, FType::Flags))
        .collect();
    for ff in &flag_fields {
        writeln!(out, "        let {} = cursor.read_u32::<LittleEndian>().map_err(|_| \"read {}\")?;",
            ff.name, ff.name).unwrap();
    }

    for field in &real_fields {
        if matches!(field.ftype, FType::Flags) { continue; }
        let sname = sanitize_name(&field.name);
        let is_cond = field.flag_field.is_some();

        if is_cond && !matches!(field.ftype, FType::True) {
            let ff_name = field.flag_field.as_ref().unwrap();
            let bit = field.flag_bit.unwrap();
            writeln!(out, "        if {} & (1 << {}) != 0 {{", ff_name, bit).unwrap();
            write_deser_field_v2(out, field.ftype, &sname, true, "            ");
            writeln!(out, "        }}").unwrap();
        } else if matches!(field.ftype, FType::True) {
            let ff_name = field.flag_field.as_ref().unwrap();
            let bit = field.flag_bit.unwrap();
            writeln!(out, "        obj.{} = {} & (1 << {}) != 0;", sname, ff_name, bit).unwrap();
        } else {
            write_deser_field_v2(out, field.ftype, &sname, false, "        ");
        }
    }

    writeln!(out, "        Ok(obj)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();
}

fn write_deser_field(out: &mut fs::File, ftype: FType, name: &str, is_option: bool, indent: &str) {
    write_deser_field_v2(out, ftype, name, is_option, indent);
}

fn write_deser_field_v2(out: &mut fs::File, ftype: FType, name: &str, is_option: bool, indent: &str) {
    let assign = if is_option {
        format!("obj.{} = Some(", name)
    } else {
        format!("obj.{} = ", name)
    };
    let close = if is_option { ");" } else { ";" };

    match ftype {
        FType::Int => writeln!(out, "{}{}cursor.read_i32::<LittleEndian>().map_err(|_| \"read {}\")?{}", indent, assign, name, close).unwrap(),
        FType::Long => writeln!(out, "{}{}cursor.read_i64::<LittleEndian>().map_err(|_| \"read {}\")?{}", indent, assign, name, close).unwrap(),
        FType::Double => writeln!(out, "{}{}f64::from_bits(cursor.read_u64::<LittleEndian>().map_err(|_| \"read {}\")?){}", indent, assign, name, close).unwrap(),
        FType::Str => writeln!(out, "{}{}deserialize_string(cursor)?{}", indent, assign, close).unwrap(),
        FType::Bytes => writeln!(out, "{}{}deserialize_bytes(cursor).map_err(|_| \"read {}\".to_string())?{}", indent, assign, name, close).unwrap(),
        FType::Bool => writeln!(out, "{}{}cursor.read_u32::<LittleEndian>().map_err(|_| \"read {}\")? == 0x997275b5{}", indent, assign, name, close).unwrap(),
        FType::VecInt => writeln!(out, "{}{}read_vector_int(cursor)?{}", indent, assign, close).unwrap(),
        FType::VecLong => writeln!(out, "{}{}read_vector_long(cursor)?{}", indent, assign, close).unwrap(),
        FType::VecStr => writeln!(out, "{}{}read_vector_string(cursor)?{}", indent, assign, close).unwrap(),
        FType::VecBytes => {
            writeln!(out, "{}{{", indent).unwrap();
            writeln!(out, "{}    let vc = cursor.read_u32::<LittleEndian>().map_err(|_| \"vec ctor\")?;", indent).unwrap();
            writeln!(out, "{}    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| \"vec cnt\")?;", indent).unwrap();
            writeln!(out, "{}    let mut v = Vec::with_capacity(cnt as usize);", indent).unwrap();
            writeln!(out, "{}    for _ in 0..cnt {{ v.push(deserialize_bytes(cursor).map_err(|_| \"vb\".to_string())?); }}", indent).unwrap();
            writeln!(out, "{}    {}v{}", indent, assign, close).unwrap();
            writeln!(out, "{}}}", indent).unwrap();
        }
        FType::Object => {
            // capture raw bytes of a TL object by recording position before/after skip
            writeln!(out, "{}{{", indent).unwrap();
            writeln!(out, "{}    let start = cursor.position() as usize;", indent).unwrap();
            writeln!(out, "{}    skip_tl(cursor)?;", indent).unwrap();
            writeln!(out, "{}    let end = cursor.position() as usize;", indent).unwrap();
            writeln!(out, "{}    let slice = cursor.get_ref();", indent).unwrap();
            writeln!(out, "{}    {}slice[start..end].to_vec(){}", indent, assign, close).unwrap();
            writeln!(out, "{}}}", indent).unwrap();
        }
        FType::VecObj => {
            // capture each object in vector as raw bytes
            writeln!(out, "{}{{", indent).unwrap();
            writeln!(out, "{}    let _vc = cursor.read_u32::<LittleEndian>().map_err(|_| \"vec ctor\")?;", indent).unwrap();
            writeln!(out, "{}    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| \"vec cnt\")?;", indent).unwrap();
            writeln!(out, "{}    let mut v = Vec::with_capacity(cnt as usize);", indent).unwrap();
            writeln!(out, "{}    for _ in 0..cnt {{", indent).unwrap();
            writeln!(out, "{}        let s = cursor.position() as usize;", indent).unwrap();
            writeln!(out, "{}        skip_tl(cursor)?;", indent).unwrap();
            writeln!(out, "{}        let e = cursor.position() as usize;", indent).unwrap();
            writeln!(out, "{}        v.push(cursor.get_ref()[s..e].to_vec());", indent).unwrap();
            writeln!(out, "{}    }}", indent).unwrap();
            writeln!(out, "{}    {}v{}", indent, assign, close).unwrap();
            writeln!(out, "{}}}", indent).unwrap();
        }
        FType::Int128 => {
            writeln!(out, "{}{{ let mut b = [0u8; 16]; cursor.read_exact(&mut b).map_err(|_| \"i128\")?; {}b{} }}", indent, assign, close).unwrap();
        }
        FType::Int256 => {
            writeln!(out, "{}{{ let mut b = [0u8; 32]; cursor.read_exact(&mut b).map_err(|_| \"i256\")?; {}b{} }}", indent, assign, close).unwrap();
        }
        _ => {}
    }
}

fn write_enum_type(out: &mut fs::File, type_ctors: &[&Constructor], type_name: &str) {
    // check no unsupported types (only SKIP is unsupported now)
    for ctor in type_ctors {
        for field in &ctor.fields {
            if matches!(field.ftype, FType::Flags | FType::True) { continue; }
            let ty = rust_owned_type(field.ftype);
            if ty == "SKIP" { return; }
        }
    }

    let enum_name = to_struct_name(type_name);

    // generate variant names from constructor names
    // e.g. "userStatusEmpty" -> "Empty", "userStatusOnline" -> "Online"
    let base = type_name.split('.').last().unwrap_or(type_name);

    writeln!(out, "#[derive(Debug, Clone)]").unwrap();
    writeln!(out, "pub enum {} {{", enum_name).unwrap();

    for ctor in type_ctors {
        let variant = variant_name(&ctor.name, base);
        let real_fields: Vec<&Field> = ctor.fields.iter()
            .filter(|f| !matches!(f.ftype, FType::Flags))
            .collect();

        if real_fields.is_empty() {
            writeln!(out, "    {},", variant).unwrap();
        } else {
            let field_defs: Vec<String> = real_fields.iter().map(|f| {
                let sname = sanitize_name(&f.name);
                let ty = rust_owned_type(f.ftype);
                if f.flag_field.is_some() && !matches!(f.ftype, FType::True) {
                    format!("{}: Option<{}>", sname, ty)
                } else {
                    format!("{}: {}", sname, ty)
                }
            }).collect();
            writeln!(out, "    {} {{ {} }},", variant, field_defs.join(", ")).unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();

    // generate deserialize
    writeln!(out, "impl {} {{", enum_name).unwrap();
    writeln!(out, "    pub fn deserialize(cursor: &mut Cursor<&[u8]>) -> Result<Self, String> {{").unwrap();
    writeln!(out, "        let ctor = cursor.read_u32::<LittleEndian>().map_err(|_| \"read ctor\")?;").unwrap();
    writeln!(out, "        Self::deserialize_by_id(cursor, ctor)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "").unwrap();
    writeln!(out, "    pub fn deserialize_by_id(cursor: &mut Cursor<&[u8]>, ctor: u32) -> Result<Self, String> {{").unwrap();
    writeln!(out, "        match ctor {{").unwrap();

    for ctor in type_ctors {
        let variant = variant_name(&ctor.name, base);
        let real_fields: Vec<&Field> = ctor.fields.iter()
            .filter(|f| !matches!(f.ftype, FType::Flags))
            .collect();
        let flag_fields: Vec<&Field> = ctor.fields.iter()
            .filter(|f| matches!(f.ftype, FType::Flags))
            .collect();

        writeln!(out, "            {:#010x} => {{", ctor.id).unwrap();

        // read flags
        for ff in &flag_fields {
            writeln!(out, "                let {} = cursor.read_u32::<LittleEndian>().map_err(|_| \"flags\")?;", ff.name).unwrap();
        }

        if real_fields.is_empty() {
            writeln!(out, "                Ok({}::{})", enum_name, variant).unwrap();
        } else {
            // read fields
            for field in &real_fields {
                let sname = sanitize_name(&field.name);
                if matches!(field.ftype, FType::True) {
                    // bool from flag bit
                    let ff_name = field.flag_field.as_ref().unwrap();
                    let bit = field.flag_bit.unwrap();
                    writeln!(out, "                let {} = {} & (1 << {}) != 0;", sname, ff_name, bit).unwrap();
                } else if field.flag_field.is_some() {
                    let ff_name = field.flag_field.as_ref().unwrap();
                    let bit = field.flag_bit.unwrap();
                    writeln!(out, "                let {} = if {} & (1 << {}) != 0 {{", sname, ff_name, bit).unwrap();
                    write_deser_expr(out, field.ftype, "                    ");
                    writeln!(out, "                }} else {{ None }};").unwrap();
                } else {
                    writeln!(out, "                let {} = {{", sname).unwrap();
                    write_deser_expr_direct(out, field.ftype, "                    ");
                    writeln!(out, "                }};").unwrap();
                }
            }
            let field_names: Vec<String> = real_fields.iter().map(|f| sanitize_name(&f.name)).collect();
            writeln!(out, "                Ok({}::{} {{ {} }})", enum_name, variant, field_names.join(", ")).unwrap();
        }
        writeln!(out, "            }}").unwrap();
    }

    writeln!(out, "            _ => Err(format!(\"unknown {} ctor {{:#x}}\", ctor)),", type_name).unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "").unwrap();
}

fn variant_name(ctor_name: &str, base_type: &str) -> String {
    // strip namespace prefix: "account.resetPasswordOk" -> "resetPasswordOk"
    let name_no_ns = if let Some(dot) = ctor_name.find('.') {
        &ctor_name[dot + 1..]
    } else {
        ctor_name
    };

    // strip common type prefix: "resetPasswordOk" with base "ResetPasswordResult" -> "Ok"
    let lower_base = base_type.to_lowercase().replace(".", "");
    let lower_name = name_no_ns.to_lowercase();

    // try to find the longest matching prefix
    let stripped = if lower_name.len() > 2 {
        // try removing base type name (without "Result"/"Full" suffix)
        let base_clean = lower_base.trim_end_matches("result")
            .trim_end_matches("full")
            .trim_end_matches("type");
        if lower_name.starts_with(base_clean) && name_no_ns.len() > base_clean.len() {
            &name_no_ns[base_clean.len()..]
        } else {
            name_no_ns
        }
    } else {
        name_no_ns
    };

    // capitalize first letter
    let mut s = String::new();
    let mut chars = stripped.chars();
    if let Some(first) = chars.next() {
        s.push(first.to_ascii_uppercase());
        s.extend(chars);
    }
    // ensure it's a valid Rust identifier (no dots, starts with uppercase letter)
    let s = s.replace('.', "_");
    // if starts with digit, prefix with V
    if s.chars().next().map_or(true, |c| c.is_ascii_digit()) {
        format!("V{}", s)
    } else if s == "Self" || s == "self" {
        "Myself".to_string()
    } else {
        s
    }
}

fn write_deser_expr(out: &mut fs::File, ftype: FType, indent: &str) {
    match ftype {
        FType::Int => writeln!(out, "{}Some(cursor.read_i32::<LittleEndian>().map_err(|_| \"int\")?)", indent).unwrap(),
        FType::Long => writeln!(out, "{}Some(cursor.read_i64::<LittleEndian>().map_err(|_| \"long\")?)", indent).unwrap(),
        FType::Double => writeln!(out, "{}Some(f64::from_bits(cursor.read_u64::<LittleEndian>().map_err(|_| \"double\")?))", indent).unwrap(),
        FType::Str => writeln!(out, "{}Some(deserialize_string(cursor)?)", indent).unwrap(),
        FType::Bytes => writeln!(out, "{}Some(deserialize_bytes(cursor).map_err(|_| \"bytes\".to_string())?)", indent).unwrap(),
        FType::Bool => writeln!(out, "{}Some(cursor.read_u32::<LittleEndian>().map_err(|_| \"bool\")? == 0x997275b5)", indent).unwrap(),
        FType::VecInt => writeln!(out, "{}Some(read_vector_int(cursor)?)", indent).unwrap(),
        FType::VecLong => writeln!(out, "{}Some(read_vector_long(cursor)?)", indent).unwrap(),
        FType::VecStr => writeln!(out, "{}Some(read_vector_string(cursor)?)", indent).unwrap(),
        FType::Object => {
            writeln!(out, "{}{{ let s = cursor.position() as usize; skip_tl(cursor)?; let e = cursor.position() as usize; Some(cursor.get_ref()[s..e].to_vec()) }}", indent).unwrap();
        }
        FType::VecObj => {
            writeln!(out, "{}{{", indent).unwrap();
            writeln!(out, "{}    let _vc = cursor.read_u32::<LittleEndian>().map_err(|_| \"vc\")?;", indent).unwrap();
            writeln!(out, "{}    let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| \"cnt\")?;", indent).unwrap();
            writeln!(out, "{}    let mut v = Vec::new();", indent).unwrap();
            writeln!(out, "{}    for _ in 0..cnt {{ let s = cursor.position() as usize; skip_tl(cursor)?; let e = cursor.position() as usize; v.push(cursor.get_ref()[s..e].to_vec()); }}", indent).unwrap();
            writeln!(out, "{}    Some(v)", indent).unwrap();
            writeln!(out, "{}}}", indent).unwrap();
        }
        FType::Int128 => writeln!(out, "{}{{ let mut b = [0u8; 16]; cursor.read_exact(&mut b).map_err(|_| \"i128\")?; Some(b) }}", indent).unwrap(),
        FType::Int256 => writeln!(out, "{}{{ let mut b = [0u8; 32]; cursor.read_exact(&mut b).map_err(|_| \"i256\")?; Some(b) }}", indent).unwrap(),
        _ => writeln!(out, "{}None // unsupported", indent).unwrap(),
    }
}

fn write_deser_expr_direct(out: &mut fs::File, ftype: FType, indent: &str) {
    match ftype {
        FType::Int => writeln!(out, "{}cursor.read_i32::<LittleEndian>().map_err(|_| \"int\")?", indent).unwrap(),
        FType::Long => writeln!(out, "{}cursor.read_i64::<LittleEndian>().map_err(|_| \"long\")?", indent).unwrap(),
        FType::Double => writeln!(out, "{}f64::from_bits(cursor.read_u64::<LittleEndian>().map_err(|_| \"double\")?)", indent).unwrap(),
        FType::Str => writeln!(out, "{}deserialize_string(cursor)?", indent).unwrap(),
        FType::Bytes => writeln!(out, "{}deserialize_bytes(cursor).map_err(|_| \"bytes\".to_string())?", indent).unwrap(),
        FType::Bool => writeln!(out, "{}cursor.read_u32::<LittleEndian>().map_err(|_| \"bool\")? == 0x997275b5", indent).unwrap(),
        FType::VecInt => writeln!(out, "{}read_vector_int(cursor)?", indent).unwrap(),
        FType::VecLong => writeln!(out, "{}read_vector_long(cursor)?", indent).unwrap(),
        FType::VecStr => writeln!(out, "{}read_vector_string(cursor)?", indent).unwrap(),
        FType::Object => {
            writeln!(out, "{}{{ let s = cursor.position() as usize; skip_tl(cursor)?; let e = cursor.position() as usize; cursor.get_ref()[s..e].to_vec() }}", indent).unwrap();
        }
        FType::VecObj => {
            writeln!(out, "{}{{ let _vc = cursor.read_u32::<LittleEndian>().map_err(|_| \"vc\")?; let cnt = cursor.read_u32::<LittleEndian>().map_err(|_| \"cnt\")?; let mut v = Vec::new(); for _ in 0..cnt {{ let s = cursor.position() as usize; skip_tl(cursor)?; let e = cursor.position() as usize; v.push(cursor.get_ref()[s..e].to_vec()); }} v }}", indent).unwrap();
        }
        FType::Int128 => writeln!(out, "{}{{ let mut b = [0u8; 16]; cursor.read_exact(&mut b).map_err(|_| \"i128\")?; b }}", indent).unwrap(),
        FType::Int256 => writeln!(out, "{}{{ let mut b = [0u8; 32]; cursor.read_exact(&mut b).map_err(|_| \"i256\")?; b }}", indent).unwrap(),
        _ => writeln!(out, "{}Default::default() // unsupported", indent).unwrap(),
    }
}

fn write_method_parsers(out: &mut fs::File, ctors: &[Constructor]) {
    writeln!(out, "// --- method response parsers ---").unwrap();
    writeln!(out, "").unwrap();

    let mut known_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut by_type: HashMap<String, Vec<&Constructor>> = HashMap::new();
    for ctor in ctors.iter().filter(|c| !c.is_function) {
        if ctor.result_type.contains(' ') || ctor.result_type.contains('<') { continue; }
        by_type.entry(ctor.result_type.clone()).or_default().push(ctor);
    }
    for (type_name, _) in &by_type {
        known_types.insert(type_name.clone());
    }

    for ctor in ctors.iter().filter(|c| c.is_function) {
        let rt = &ctor.result_type;
        let fn_name = format!("parse_{}", ctor.name.replace('.', "_"));

        // Bool result type
        if rt == "Bool" {
            writeln!(out, "pub fn {}(data: &[u8]) -> Result<bool, String> {{", fn_name).unwrap();
            writeln!(out, "    parse_bool(data)").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
            continue;
        }

        // Vector<int>
        if rt == "Vector<int>" {
            writeln!(out, "pub fn {}(data: &[u8]) -> Result<Vec<i32>, String> {{", fn_name).unwrap();
            writeln!(out, "    parse_vector_int_response(data)").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
            continue;
        }

        // Vector<long>
        if rt == "Vector<long>" {
            writeln!(out, "pub fn {}(data: &[u8]) -> Result<Vec<i64>, String> {{", fn_name).unwrap();
            writeln!(out, "    parse_vector_long_response(data)").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
            continue;
        }

        // Vector<T> where T is a known type
        if rt.starts_with("Vector<") && rt.ends_with('>') {
            let inner_type = &rt[7..rt.len()-1];
            if inner_type == "string" {
                writeln!(out, "pub fn {}(data: &[u8]) -> Result<Vec<String>, String> {{", fn_name).unwrap();
                writeln!(out, "    let inner = unwrap_rpc(data)?;").unwrap();
                writeln!(out, "    let mut cursor = Cursor::new(inner.as_slice());").unwrap();
                writeln!(out, "    read_vector_string(&mut cursor)").unwrap();
                writeln!(out, "}}").unwrap();
                writeln!(out, "").unwrap();
            } else if known_types.contains(inner_type) {
                let inner_struct = to_struct_name(inner_type);
                writeln!(out, "pub fn {}(data: &[u8]) -> Result<Vec<{}>, String> {{", fn_name, inner_struct).unwrap();
                writeln!(out, "    parse_vector_response::<{}>(data)", inner_struct).unwrap();
                writeln!(out, "}}").unwrap();
                writeln!(out, "").unwrap();
            }
            continue;
        }

        // skip generic/unknown
        if rt.contains(' ') || rt.contains('<') { continue; }
        if rt == "X" { continue; }

        // Updates — generate parser that returns raw bytes (too complex for full deser)
        if rt == "Updates" {
            writeln!(out, "pub fn {}(data: &[u8]) -> Result<Vec<u8>, String> {{", fn_name).unwrap();
            writeln!(out, "    unwrap_rpc(data)").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
            continue;
        }

        if !known_types.contains(rt) { continue; }

        let struct_name = to_struct_name(rt);
        let type_ctors = by_type.get(rt).unwrap();

        if type_ctors.len() == 1 {
            writeln!(out, "pub fn {}(data: &[u8]) -> Result<{}, String> {{", fn_name, struct_name).unwrap();
            writeln!(out, "    let inner = unwrap_rpc(data)?;").unwrap();
            writeln!(out, "    let mut cursor = Cursor::new(inner.as_slice());").unwrap();
            writeln!(out, "    let _ctor = cursor.read_u32::<LittleEndian>().map_err(|_| \"ctor\")?;").unwrap();
            writeln!(out, "    {}::deserialize(&mut cursor)", struct_name).unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
        } else {
            writeln!(out, "pub fn {}(data: &[u8]) -> Result<{}, String> {{", fn_name, struct_name).unwrap();
            writeln!(out, "    let inner = unwrap_rpc(data)?;").unwrap();
            writeln!(out, "    let mut cursor = Cursor::new(inner.as_slice());").unwrap();
            writeln!(out, "    {}::deserialize(&mut cursor)", struct_name).unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
        }
    }
}

fn write_trait_impls(out: &mut fs::File, ctors: &[Constructor]) {
    // generate TlDeserialize impls for all generated types
    let mut by_type: HashMap<String, Vec<&Constructor>> = HashMap::new();
    for ctor in ctors.iter().filter(|c| !c.is_function) {
        if ctor.result_type.contains(' ') || ctor.result_type.contains('<') { continue; }
        by_type.entry(ctor.result_type.clone()).or_default().push(ctor);
    }

    writeln!(out, "// --- TlDeserialize trait impls ---").unwrap();
    writeln!(out, "").unwrap();

    for (type_name, type_ctors) in &by_type {
        let struct_name = to_struct_name(type_name);

        if type_ctors.len() == 1 {
            // single-ctor struct: TlDeserialize reads ctor then calls deserialize
            // but our struct::deserialize doesn't read ctor, so we need to handle it
            writeln!(out, "impl TlDeserialize for {} {{", struct_name).unwrap();
            writeln!(out, "    fn tl_deserialize(cursor: &mut Cursor<&[u8]>) -> Result<Self, String> {{").unwrap();
            writeln!(out, "        let _ctor = cursor.read_u32::<LittleEndian>().map_err(|_| \"ctor\")?;").unwrap();
            writeln!(out, "        Self::deserialize(cursor)").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
        } else {
            // multi-ctor enum: TlDeserialize calls deserialize which reads ctor internally
            writeln!(out, "impl TlDeserialize for {} {{", struct_name).unwrap();
            writeln!(out, "    fn tl_deserialize(cursor: &mut Cursor<&[u8]>) -> Result<Self, String> {{").unwrap();
            writeln!(out, "        Self::deserialize(cursor)").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "").unwrap();
        }
    }
}
