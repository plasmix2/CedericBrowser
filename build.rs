use std::{env, fs, path::PathBuf};

fn push_header(out: &mut Vec<u8>, data_size: u32, kind: u16, id: u16, flags: u16, lang: u16) {
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src/ui/logo.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Ok(ico) = fs::read("src/ui/logo.ico") else { return };
    if ico.len() < 6 {
        return;
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    if ico.len() < 6 + count * 16 {
        return;
    }

    let mut res = Vec::new();
    // 32-byte null resource header.
    push_header(&mut res, 0, 0, 0, 0, 0);

    let mut grp = Vec::with_capacity(6 + count * 14);
    grp.extend_from_slice(&ico[0..6]);

    for i in 0..count {
        let ent = &ico[6 + i * 16..6 + (i + 1) * 16];
        let size = u32::from_le_bytes([ent[8], ent[9], ent[10], ent[11]]) as usize;
        let offset = u32::from_le_bytes([ent[12], ent[13], ent[14], ent[15]]) as usize;
        if offset + size > ico.len() {
            return;
        }
        let id = (i + 1) as u16;
        push_header(&mut res, size as u32, 3, id, 0x1010, 0x0409);
        res.extend_from_slice(&ico[offset..offset + size]);
        pad4(&mut res);

        grp.extend_from_slice(&ent[0..12]);
        grp.extend_from_slice(&id.to_le_bytes());
    }

    push_header(&mut res, grp.len() as u32, 14, 1, 0x1030, 0x0409);
    res.extend_from_slice(&grp);
    pad4(&mut res);

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let res_path = out_dir.join("persona_icon.res");
    if fs::write(&res_path, &res).is_ok() {
        println!("cargo:rustc-link-arg-bins={}", res_path.display());
    }
}
