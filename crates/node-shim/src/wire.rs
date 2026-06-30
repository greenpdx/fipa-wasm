//! The signed message envelope and its length-prefixed codec (R1) — identical wire
//! format to the full node, so a shim device and a hosted node interoperate.

/// A message in flight between nodes. `from_addr` is the sender's return address;
/// `nonce`/`sig`/`sender_pub` authenticate it.
#[derive(Clone, Debug, Default)]
pub struct NodeMsg {
    pub to: String,
    pub from: String,
    pub from_addr: String,
    pub unl: Vec<u8>,
    pub body: Vec<u8>,
    pub nonce: Vec<u8>,
    pub sig: Vec<u8>,
    pub sender_pub: Vec<u8>,
}

/// Frame kind: an application message (the only kind the shim needs).
pub const KIND_MSG: u8 = 1;

fn put(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
}

fn get(buf: &[u8], p: &mut usize) -> Option<Vec<u8>> {
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
