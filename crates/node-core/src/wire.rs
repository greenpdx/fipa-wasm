//! The signed message envelope and its length-prefixed codec (R1). The exact wire
//! format both the full node and the embedded shim speak.

/// A message in flight between nodes. `from_addr` is the sender's return address;
/// `nonce`/`sig`/`sender_pub` authenticate it.
#[derive(Clone, Debug, Default)]
pub struct NodeMsg {
    pub to: String,
    pub from: String,
    pub from_addr: String,
    pub unl: Vec<u8>,
    pub body: Vec<u8>,
    /// Anti-replay nonce (16 bytes when signed).
    pub nonce: Vec<u8>,
    /// Ed25519 signature over [`signing_bytes`] (64 bytes when signed).
    pub sig: Vec<u8>,
    /// The signing node's public key (32 bytes when signed).
    pub sender_pub: Vec<u8>,
}

fn put(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
}

fn get(buf: &[u8], p: &mut usize) -> Option<Vec<u8>> {
    // checked arithmetic: a length near usize::MAX cannot wrap past the bounds
    // check into an out-of-range slice on a 32-bit target.
    if p.checked_add(4)? > buf.len() {
        return None;
    }
    let n = u32::from_be_bytes(buf[*p..*p + 4].try_into().ok()?) as usize;
    *p += 4;
    if p.checked_add(n)? > buf.len() {
        return None;
    }
    let v = buf[*p..*p + n].to_vec();
    *p += n;
    Some(v)
}

pub fn encode_msg(m: &NodeMsg) -> Vec<u8> {
    let mut b = Vec::new();
    put(&mut b, m.to.as_bytes());
    put(&mut b, m.from.as_bytes());
    put(&mut b, m.from_addr.as_bytes());
    put(&mut b, &m.unl);
    put(&mut b, &m.body);
    put(&mut b, &m.nonce);
    put(&mut b, &m.sig);
    put(&mut b, &m.sender_pub);
    b
}

pub fn decode_msg(p: &[u8]) -> Option<NodeMsg> {
    let mut i = 0;
    Some(NodeMsg {
        to: String::from_utf8(get(p, &mut i)?).ok()?,
        from: String::from_utf8(get(p, &mut i)?).ok()?,
        from_addr: String::from_utf8(get(p, &mut i)?).ok()?,
        unl: get(p, &mut i)?,
        body: get(p, &mut i)?,
        nonce: get(p, &mut i)?,
        sig: get(p, &mut i)?,
        sender_pub: get(p, &mut i)?,
    })
}

/// The exact bytes covered by the signature: every field **except** `sig`.
pub fn signing_bytes(m: &NodeMsg) -> Vec<u8> {
    let mut b = Vec::new();
    put(&mut b, m.to.as_bytes());
    put(&mut b, m.from.as_bytes());
    put(&mut b, m.from_addr.as_bytes());
    put(&mut b, &m.unl);
    put(&mut b, &m.body);
    put(&mut b, &m.nonce);
    put(&mut b, &m.sender_pub);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_roundtrips() {
        let m = NodeMsg {
            to: "dst".into(),
            from: "src".into(),
            from_addr: "127.0.0.1:9".into(),
            unl: b"obj(ping, x)".to_vec(),
            body: b"payload".to_vec(),
            nonce: vec![1; 16],
            sig: vec![2; 64],
            sender_pub: vec![3; 32],
        };
        let back = decode_msg(&encode_msg(&m)).unwrap();
        assert_eq!(back.to, m.to);
        assert_eq!(back.body, m.body);
        assert_eq!(back.sender_pub, m.sender_pub);
        // signing bytes exclude the signature
        assert_eq!(signing_bytes(&m), signing_bytes(&back));
    }

    #[test]
    fn truncated_input_is_rejected_not_panicked() {
        assert!(decode_msg(&[0, 0, 0, 9, 1, 2]).is_none()); // claims 9 bytes, has 2
    }
}
