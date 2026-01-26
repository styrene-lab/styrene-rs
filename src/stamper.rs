use hkdf::Hkdf;
use sha2::Sha256;

pub fn stamp_workblock(material: &[u8], expand_rounds: usize) -> Vec<u8> {
    let mut workblock = Vec::with_capacity(expand_rounds * 256);

    for n in 0..expand_rounds {
        let mut salt_data = Vec::with_capacity(material.len() + 8);
        salt_data.extend_from_slice(material);
        let packed = rmp_serde::to_vec(&n).unwrap();
        salt_data.extend_from_slice(&packed);
        let salt_hash = reticulum::hash::Hash::new_from_slice(&salt_data);

        let hk = Hkdf::<Sha256>::new(Some(salt_hash.as_slice()), material);
        let mut okm = [0u8; 256];
        hk.expand(&[], &mut okm).unwrap();
        workblock.extend_from_slice(&okm);
    }

    workblock
}

pub fn stamp_value(workblock: &[u8], stamp: &[u8]) -> u32 {
    let hash = reticulum::hash::Hash::new_from_slice(&[workblock, stamp].concat());
    let mut value = 0u32;

    for byte in hash.as_slice() {
        if *byte == 0 {
            value += 8;
        } else {
            value += byte.leading_zeros();
            break;
        }
    }

    value
}

pub fn stamp_valid(stamp: &[u8], target_cost: u32, workblock: &[u8]) -> bool {
    stamp_value(workblock, stamp) >= target_cost
}
