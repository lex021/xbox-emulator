use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

use super::wfp_codec::{struct_codec, Codec, R, W};
use super::wfp_names;

extern "system" {
    fn CreateFileMappingW(file: *mut c_void, sa: *mut c_void, protect: u32, high: u32, low: u32, name: *const u16) -> *mut c_void;
    fn MapViewOfFile(h: *mut c_void, access: u32, high: u32, low: u32, n: usize) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
}

pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const FWP_E_CALLOUT_NOT_FOUND: u32 = 0x8032_0001;
pub const FWP_E_FILTER_NOT_FOUND: u32 = 0x8032_0003;
pub const FWP_E_LAYER_NOT_FOUND: u32 = 0x8032_0004;
pub const FWP_E_PROVIDER_NOT_FOUND: u32 = 0x8032_0005;
pub const FWP_E_PROVIDER_CONTEXT_NOT_FOUND: u32 = 0x8032_0006;
pub const FWP_E_SUBLAYER_NOT_FOUND: u32 = 0x8032_0007;
pub const FWP_E_ALREADY_EXISTS: u32 = 0x8032_0009;
pub const FWP_E_IN_USE: u32 = 0x8032_000A;
pub const FWP_E_DYNAMIC_SESSION_IN_PROGRESS: u32 = 0x8032_000B;
pub const FWP_E_NO_TXN_IN_PROGRESS: u32 = 0x8032_000D;
pub const FWP_E_TXN_IN_PROGRESS: u32 = 0x8032_000E;

pub const FWPM_SESSION_FLAG_DYNAMIC: u32 = 0x1;
pub const FWPM_PROVIDER_FLAG_PERSISTENT: u32 = 0x1;
pub const FWPM_SUBLAYER_FLAG_PERSISTENT: u32 = 0x1;
pub const FWPM_CALLOUT_FLAG_PERSISTENT: u32 = 0x1_0000;
pub const FWPM_FILTER_FLAG_PERSISTENT: u32 = 0x1;
pub const FWPM_FILTER_FLAG_HAS_PROVIDER_CONTEXT: u32 = 0x4;
pub const FWPM_PROVIDER_CONTEXT_FLAG_PERSISTENT: u32 = 0x1;
pub const FWP_ACTION_FLAG_CALLOUT: u32 = 0x4000;

pub const FWPM_IPSEC_KEYING_CONTEXT: u32 = 0;
pub const FWPM_CLASSIFY_OPTIONS_CONTEXT: u32 = 7;
pub const FWPM_GENERAL_CONTEXT: u32 = 8;
pub const FWPM_PROVIDER_CONTEXT_TYPE_MAX: u32 = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Provider,
    ProviderContext,
    SubLayer,
    Callout,
    Filter,
    IpsecSaContext,
    IpsecSa,
    IkeSa,
    AleEndpoint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Provider {
    pub key: [u8; 16],
    pub name: Option<Vec<u16>>,
    pub description: Option<Vec<u16>>,
    pub flags: u32,
    pub data: Vec<u8>,
    pub service_name: Option<Vec<u16>>,
}
struct_codec!(Provider { key, name, description, flags, data, service_name });

#[derive(Clone, Debug, PartialEq)]
pub struct SubLayer {
    pub key: [u8; 16],
    pub name: Option<Vec<u16>>,
    pub description: Option<Vec<u16>>,
    pub flags: u32,
    pub provider_key: Option<[u8; 16]>,
    pub data: Vec<u8>,
    pub weight: u16,
}
struct_codec!(SubLayer { key, name, description, flags, provider_key, data, weight });

#[derive(Clone, Debug, PartialEq)]
pub struct Callout {
    pub key: [u8; 16],
    pub name: Option<Vec<u16>>,
    pub description: Option<Vec<u16>>,
    pub flags: u32,
    pub provider_key: Option<[u8; 16]>,
    pub data: Vec<u8>,
    pub layer: [u8; 16],
    pub id: u32,
}
struct_codec!(Callout { key, name, description, flags, provider_key, data, layer, id });

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Empty,
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Bytes16([u8; 16]),
    Blob(Vec<u8>),
    Sid(Vec<u8>),
    SecurityDescriptor(Vec<u8>),
    TokenInformation(bool),
    TokenAccessInformation(bool),
    Unicode(Option<Vec<u16>>),
    Bytes6([u8; 6]),
    V4AddrMask(u32, u32),
    V6AddrMask([u8; 16], u8),
    Range(Box<Value>, Box<Value>),
    Other(u32),
}

impl Value {
    pub fn data_type(&self) -> u32 {
        match self {
            Value::Empty => 0,
            Value::U8(_) => 1,
            Value::U16(_) => 2,
            Value::U32(_) => 3,
            Value::U64(_) => 4,
            Value::I8(_) => 5,
            Value::I16(_) => 6,
            Value::I32(_) => 7,
            Value::I64(_) => 8,
            Value::F32(_) => 9,
            Value::F64(_) => 10,
            Value::Bytes16(_) => 11,
            Value::Blob(_) => 12,
            Value::Sid(_) => 13,
            Value::SecurityDescriptor(_) => 14,
            Value::TokenInformation(_) => 15,
            Value::TokenAccessInformation(_) => 16,
            Value::Unicode(_) => 17,
            Value::Bytes6(_) => 18,
            Value::V4AddrMask(..) => 0x100,
            Value::V6AddrMask(..) => 0x101,
            Value::Range(..) => 0x102,
            Value::Other(t) => *t,
        }
    }
}

impl Codec for Value {
    fn put(&self, w: &mut W) {
        self.data_type().put(w);
        match self {
            Value::Empty => {}
            Value::U8(v) => v.put(w),
            Value::U16(v) => v.put(w),
            Value::U32(v) => v.put(w),
            Value::U64(v) => v.put(w),
            Value::I8(v) => v.put(w),
            Value::I16(v) => v.put(w),
            Value::I32(v) => v.put(w),
            Value::I64(v) => v.put(w),
            Value::F32(v) => v.put(w),
            Value::F64(v) => v.put(w),
            Value::Bytes16(v) => v.put(w),
            Value::Blob(v) | Value::Sid(v) | Value::SecurityDescriptor(v) => v.put(w),
            Value::TokenInformation(v) | Value::TokenAccessInformation(v) => v.put(w),
            Value::Unicode(v) => v.put(w),
            Value::Bytes6(v) => v.put(w),
            Value::V4AddrMask(a, m) => {
                a.put(w);
                m.put(w);
            }
            Value::V6AddrMask(a, p) => {
                a.put(w);
                p.put(w);
            }
            Value::Range(l, h) => {
                l.put(w);
                h.put(w);
            }
            Value::Other(_) => {}
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(match u32::get(r)? {
            0 => Value::Empty,
            1 => Value::U8(Codec::get(r)?),
            2 => Value::U16(Codec::get(r)?),
            3 => Value::U32(Codec::get(r)?),
            4 => Value::U64(Codec::get(r)?),
            5 => Value::I8(Codec::get(r)?),
            6 => Value::I16(Codec::get(r)?),
            7 => Value::I32(Codec::get(r)?),
            8 => Value::I64(Codec::get(r)?),
            9 => Value::F32(Codec::get(r)?),
            10 => Value::F64(Codec::get(r)?),
            11 => Value::Bytes16(Codec::get(r)?),
            12 => Value::Blob(Codec::get(r)?),
            13 => Value::Sid(Codec::get(r)?),
            14 => Value::SecurityDescriptor(Codec::get(r)?),
            15 => Value::TokenInformation(Codec::get(r)?),
            16 => Value::TokenAccessInformation(Codec::get(r)?),
            17 => Value::Unicode(Codec::get(r)?),
            18 => Value::Bytes6(Codec::get(r)?),
            0x100 => Value::V4AddrMask(Codec::get(r)?, Codec::get(r)?),
            0x101 => Value::V6AddrMask(Codec::get(r)?, Codec::get(r)?),
            0x102 => Value::Range(Codec::get(r)?, Codec::get(r)?),
            t => Value::Other(t),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub field: [u8; 16],
    pub match_type: u32,
    pub value: Value,
}
struct_codec!(Condition { field, match_type, value });

#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub key: [u8; 16],
    pub name: Option<Vec<u16>>,
    pub description: Option<Vec<u16>>,
    pub flags: u32,
    pub provider_key: Option<[u8; 16]>,
    pub data: Vec<u8>,
    pub layer: [u8; 16],
    pub sublayer: [u8; 16],
    pub weight: Value,
    pub conditions: Vec<Condition>,
    pub action_type: u32,
    pub action_key: [u8; 16],
    pub context: [u8; 16],
    pub reserved: Option<[u8; 16]>,
    pub id: u64,
    pub effective_weight: Value,
}
struct_codec!(Filter { key, name, description, flags, provider_key, data, layer, sublayer, weight, conditions, action_type, action_key, context, reserved, id, effective_weight });

#[derive(Clone, Debug, PartialEq)]
pub struct AuthTransform {
    pub auth_type: u32,
    pub auth_config: u8,
    pub crypto_module: Option<[u8; 16]>,
}
struct_codec!(AuthTransform { auth_type, auth_config, crypto_module });

#[derive(Clone, Debug, PartialEq)]
pub struct CipherTransform {
    pub cipher_type: u32,
    pub cipher_config: u8,
    pub crypto_module: Option<[u8; 16]>,
}
struct_codec!(CipherTransform { cipher_type, cipher_config, crypto_module });

#[derive(Clone, Debug, PartialEq)]
pub struct SaTransform {
    pub kind: u32,
    pub auth: AuthTransform,
    pub cipher: CipherTransform,
}
struct_codec!(SaTransform { kind, auth, cipher });

#[derive(Clone, Debug, PartialEq)]
pub struct IpsecProposal {
    pub lifetime: [u32; 3],
    pub transforms: Vec<SaTransform>,
    pub pfs_group: u32,
}
struct_codec!(IpsecProposal { lifetime, transforms, pfs_group });

#[derive(Clone, Debug, PartialEq)]
pub struct CertName {
    pub kind: u32,
    pub name: Option<Vec<u16>>,
}
struct_codec!(CertName { kind, name });

#[derive(Clone, Debug, PartialEq)]
pub struct CertCriteria {
    pub data: Vec<u8>,
    pub hash: Vec<u8>,
    pub eku: Option<Option<Vec<Vec<u8>>>>,
    pub name: Option<CertName>,
    pub flags: u32,
}
struct_codec!(CertCriteria { data, hash, eku, name, flags });

#[derive(Clone, Debug, PartialEq)]
pub struct CertAuth {
    pub inbound_type: u32,
    pub inbound: Vec<CertCriteria>,
    pub outbound_type: u32,
    pub outbound: Vec<CertCriteria>,
    pub flags: u32,
    pub url: Vec<u8>,
}
struct_codec!(CertAuth { inbound_type, inbound, outbound_type, outbound, flags, url });

#[derive(Clone, Debug, PartialEq)]
pub struct AuthMethod {
    pub kind: u32,
    pub psk: Vec<u8>,
    pub psk_flags: u32,
    pub cert: CertAuth,
}
struct_codec!(AuthMethod { kind, psk, psk_flags, cert });

#[derive(Clone, Debug, PartialEq)]
pub struct EmPolicy {
    pub methods: Vec<AuthMethod>,
    pub impersonation: u32,
}
struct_codec!(EmPolicy { methods, impersonation });

#[derive(Clone, Debug, PartialEq)]
pub struct TransportPolicy {
    pub proposals: Vec<IpsecProposal>,
    pub flags: u32,
    pub nd_allow_clear_timeout: u32,
    pub idle_timeout: u32,
    pub idle_timeout_failover: u32,
    pub em_policy: Option<EmPolicy>,
}
struct_codec!(TransportPolicy { proposals, flags, nd_allow_clear_timeout, idle_timeout, idle_timeout_failover, em_policy });

#[derive(Clone, Debug, PartialEq)]
pub struct IkeProposal {
    pub cipher: u32,
    pub key_len: u32,
    pub rounds: u32,
    pub integrity: u32,
    pub max_lifetime: u32,
    pub dh_group: u32,
    pub qm_limit: u32,
}
struct_codec!(IkeProposal { cipher, key_len, rounds, integrity, max_lifetime, dh_group, qm_limit });

#[derive(Clone, Debug, PartialEq)]
pub struct IkePolicy {
    pub soft_expiration: u32,
    pub methods: Vec<AuthMethod>,
    pub impersonation: u32,
    pub proposals: Vec<IkeProposal>,
    pub flags: u32,
    pub max_dynamic_filters: u32,
    pub retransmit_secs: u32,
}
struct_codec!(IkePolicy { soft_expiration, methods, impersonation, proposals, flags, max_dynamic_filters, retransmit_secs });

#[derive(Clone, Debug, PartialEq)]
pub struct ClassifyOption {
    pub kind: u32,
    pub value: Value,
}
struct_codec!(ClassifyOption { kind, value });

#[derive(Clone, Debug, PartialEq)]
pub enum ContextBody {
    Null,
    Keying { key_mods: Vec<[u8; 16]>, flags: u32 },
    Transport(TransportPolicy),
    MainMode(IkePolicy),
    ClassifyOptions(Vec<ClassifyOption>),
    General(Vec<u8>),
    Opaque,
}

impl Codec for ContextBody {
    fn put(&self, w: &mut W) {
        match self {
            ContextBody::Null => 0u8.put(w),
            ContextBody::Keying { key_mods, flags } => {
                1u8.put(w);
                key_mods.put(w);
                flags.put(w);
            }
            ContextBody::Transport(t) => {
                2u8.put(w);
                t.put(w);
            }
            ContextBody::MainMode(m) => {
                3u8.put(w);
                m.put(w);
            }
            ContextBody::ClassifyOptions(o) => {
                4u8.put(w);
                o.put(w);
            }
            ContextBody::General(g) => {
                5u8.put(w);
                g.put(w);
            }
            ContextBody::Opaque => 6u8.put(w),
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(match u8::get(r)? {
            0 => ContextBody::Null,
            1 => ContextBody::Keying { key_mods: Codec::get(r)?, flags: Codec::get(r)? },
            2 => ContextBody::Transport(Codec::get(r)?),
            3 => ContextBody::MainMode(Codec::get(r)?),
            4 => ContextBody::ClassifyOptions(Codec::get(r)?),
            5 => ContextBody::General(Codec::get(r)?),
            6 => ContextBody::Opaque,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderContext {
    pub key: [u8; 16],
    pub name: Option<Vec<u16>>,
    pub description: Option<Vec<u16>>,
    pub flags: u32,
    pub provider_key: Option<[u8; 16]>,
    pub data: Vec<u8>,
    pub kind: u32,
    pub body: ContextBody,
    pub id: u64,
}
struct_codec!(ProviderContext { key, name, description, flags, provider_key, data, kind, body, id });

#[derive(Clone, Debug, PartialEq)]
pub struct AleEndpoint {
    pub id: u64,
    pub ip_version: u32,
    pub local: [u8; 16],
    pub remote: [u8; 16],
    pub protocol: u8,
    pub local_port: u16,
    pub remote_port: u16,
    pub token_id: u64,
    pub mm_sa: u64,
    pub qm_sa: u64,
    pub ipsec_status: u32,
    pub flags: u32,
    pub app_id: Vec<u8>,
}
struct_codec!(AleEndpoint { id, ip_version, local, remote, protocol, local_port, remote_port, token_id, mm_sa, qm_sa, ipsec_status, flags, app_id });

#[derive(Clone, Debug, PartialEq)]
pub struct Traffic {
    pub ip_version: u32,
    pub local: [u8; 16],
    pub remote: [u8; 16],
    pub traffic_type: u32,
    pub filter_or_policy_id: u64,
    pub remote_port: u16,
    pub local_port: u16,
    pub protocol: u8,
    pub local_if_luid: u64,
    pub real_if_profile_id: u32,
}
struct_codec!(Traffic { ip_version, local, remote, traffic_type, filter_or_policy_id, remote_port, local_port, protocol, local_if_luid, real_if_profile_id });

#[derive(Clone, Debug, PartialEq)]
pub struct IpsecToken {
    pub kind: u32,
    pub principal: u32,
    pub mode: u32,
    pub token: u64,
}
struct_codec!(IpsecToken { kind, principal, mode, token });

#[derive(Clone, Debug, PartialEq)]
pub struct IpsecId {
    pub mm_target: Option<Vec<u16>>,
    pub em_target: Option<Vec<u16>>,
    pub tokens: Vec<IpsecToken>,
    pub explicit_credentials: u64,
    pub logon_id: u64,
}
struct_codec!(IpsecId { mm_target, em_target, tokens, explicit_credentials, logon_id });

#[derive(Clone, Debug, PartialEq)]
pub struct Sa {
    pub spi: u32,
    pub transform: u32,
    pub auth: AuthTransform,
    pub auth_key: Vec<u8>,
    pub cipher: CipherTransform,
    pub cipher_key: Vec<u8>,
}
struct_codec!(Sa { spi, transform, auth, auth_key, cipher, cipher_key });

#[derive(Clone, Debug, PartialEq)]
pub struct KeyModuleState {
    pub key: [u8; 16],
    pub state: Vec<u8>,
}
struct_codec!(KeyModuleState { key, state });

#[derive(Clone, Debug, PartialEq)]
pub struct SaBundle {
    pub flags: u32,
    pub lifetime: [u32; 3],
    pub idle_timeout: u32,
    pub nd_allow_clear_timeout: u32,
    pub ipsec_id: Option<IpsecId>,
    pub nap_context: u32,
    pub qm_sa_id: u32,
    pub sas: Vec<Sa>,
    pub key_module: Option<KeyModuleState>,
    pub ip_version: u32,
    pub peer_v4_private: u32,
    pub mm_sa_id: u64,
    pub pfs_group: u32,
    pub sa_lookup_context: [u8; 16],
    pub qm_filter_id: u64,
}
struct_codec!(SaBundle { flags, lifetime, idle_timeout, nd_allow_clear_timeout, ipsec_id, nap_context, qm_sa_id, sas, key_module, ip_version, peer_v4_private, mm_sa_id, pfs_group, sa_lookup_context, qm_filter_id });

#[derive(Clone, Debug, PartialEq)]
pub struct SaDetails {
    pub ip_version: u32,
    pub direction: u32,
    pub traffic: Traffic,
    pub bundle: SaBundle,
    pub udp_encap: Option<(u16, u16)>,
    pub transport_filter: Option<Box<Filter>>,
}
struct_codec!(SaDetails { ip_version, direction, traffic, bundle, udp_encap, transport_filter });

#[derive(Clone, Debug, PartialEq)]
pub struct SaContext {
    pub id: u64,
    pub inbound: SaDetails,
    pub outbound: SaDetails,
}
struct_codec!(SaContext { id, inbound, outbound });

#[derive(Clone, Debug, PartialEq)]
pub enum CredentialBody {
    None,
    PresharedKey { key: Vec<u8>, flags: u32 },
    Certificate { subject: Vec<u8>, hash: Vec<u8>, flags: u32, certificate: Vec<u8> },
    Name(Option<Vec<u16>>),
}

impl Codec for CredentialBody {
    fn put(&self, w: &mut W) {
        match self {
            CredentialBody::None => 0u8.put(w),
            CredentialBody::PresharedKey { key, flags } => {
                1u8.put(w);
                key.put(w);
                flags.put(w);
            }
            CredentialBody::Certificate { subject, hash, flags, certificate } => {
                2u8.put(w);
                subject.put(w);
                hash.put(w);
                flags.put(w);
                certificate.put(w);
            }
            CredentialBody::Name(n) => {
                3u8.put(w);
                n.put(w);
            }
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(match u8::get(r)? {
            0 => CredentialBody::None,
            1 => CredentialBody::PresharedKey { key: Codec::get(r)?, flags: Codec::get(r)? },
            2 => CredentialBody::Certificate { subject: Codec::get(r)?, hash: Codec::get(r)?, flags: Codec::get(r)?, certificate: Codec::get(r)? },
            3 => CredentialBody::Name(Codec::get(r)?),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Credential {
    pub kind: u32,
    pub impersonation: u32,
    pub body: CredentialBody,
}
struct_codec!(Credential { kind, impersonation, body });

#[derive(Clone, Debug, PartialEq)]
pub struct IkeSa {
    pub id: u64,
    pub key_module: u32,
    pub ip_version: u32,
    pub udp_encap: Option<(u16, u16)>,
    pub traffic_ip_version: u32,
    pub local: [u8; 16],
    pub remote: [u8; 16],
    pub auth_ip_filter_id: u64,
    pub proposal: IkeProposal,
    pub cookie_initiator: u64,
    pub cookie_responder: u64,
    pub credentials: Vec<(Credential, Credential)>,
    pub policy_key: [u8; 16],
    pub virtual_if_tunnel_id: u64,
    pub correlation_key: Vec<u8>,
}
struct_codec!(IkeSa { id, key_module, ip_version, udp_encap, traffic_ip_version, local, remote, auth_ip_filter_id, proposal, cookie_initiator, cookie_responder, credentials, policy_key, virtual_if_tunnel_id, correlation_key });

#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Provider(Provider),
    SubLayer(SubLayer),
    Callout(Callout),
    Filter(Filter),
    ProviderContext(ProviderContext),
    AleEndpoint(AleEndpoint),
    SaContext(SaContext),
    IpsecSa(SaDetails),
    IkeSa(IkeSa),
}

impl Codec for Object {
    fn put(&self, w: &mut W) {
        match self {
            Object::Provider(v) => {
                0u8.put(w);
                v.put(w);
            }
            Object::SubLayer(v) => {
                1u8.put(w);
                v.put(w);
            }
            Object::Callout(v) => {
                2u8.put(w);
                v.put(w);
            }
            Object::Filter(v) => {
                3u8.put(w);
                v.put(w);
            }
            Object::ProviderContext(v) => {
                4u8.put(w);
                v.put(w);
            }
            Object::AleEndpoint(v) => {
                5u8.put(w);
                v.put(w);
            }
            Object::SaContext(v) => {
                6u8.put(w);
                v.put(w);
            }
            Object::IpsecSa(v) => {
                7u8.put(w);
                v.put(w);
            }
            Object::IkeSa(v) => {
                8u8.put(w);
                v.put(w);
            }
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(match u8::get(r)? {
            0 => Object::Provider(Codec::get(r)?),
            1 => Object::SubLayer(Codec::get(r)?),
            2 => Object::Callout(Codec::get(r)?),
            3 => Object::Filter(Codec::get(r)?),
            4 => Object::ProviderContext(Codec::get(r)?),
            5 => Object::AleEndpoint(Codec::get(r)?),
            6 => Object::SaContext(Codec::get(r)?),
            7 => Object::IpsecSa(Codec::get(r)?),
            8 => Object::IkeSa(Codec::get(r)?),
            _ => return None,
        })
    }
}

impl Object {
    fn key(&self) -> Option<[u8; 16]> {
        match self {
            Object::Provider(v) => Some(v.key),
            Object::SubLayer(v) => Some(v.key),
            Object::Callout(v) => Some(v.key),
            Object::Filter(v) => Some(v.key),
            Object::ProviderContext(v) => Some(v.key),
            _ => None,
        }
    }
    fn kind(&self) -> Kind {
        match self {
            Object::Provider(_) => Kind::Provider,
            Object::SubLayer(_) => Kind::SubLayer,
            Object::Callout(_) => Kind::Callout,
            Object::Filter(_) => Kind::Filter,
            Object::ProviderContext(_) => Kind::ProviderContext,
            Object::AleEndpoint(_) => Kind::AleEndpoint,
            Object::SaContext(_) => Kind::IpsecSaContext,
            Object::IpsecSa(_) => Kind::IpsecSa,
            Object::IkeSa(_) => Kind::IkeSa,
        }
    }
    fn provider_key(&self) -> Option<[u8; 16]> {
        match self {
            Object::SubLayer(v) => v.provider_key,
            Object::Callout(v) => v.provider_key,
            Object::Filter(v) => v.provider_key,
            Object::ProviderContext(v) => v.provider_key,
            _ => None,
        }
    }
}

struct Stored {
    object: Object,
    owner: Option<u64>,
}
struct_codec!(Stored { object, owner });

struct EnumState {
    engine: u64,
    items: Vec<Object>,
    pos: u32,
}
struct_codec!(EnumState { engine, items, pos });

struct Session {
    handle: u64,
    flags: u32,
    txn: bool,
}
struct_codec!(Session { handle, flags, txn });

struct Store {
    next: u64,
    sessions: Vec<Session>,
    objects: Vec<Stored>,
    enums: Vec<(u64, EnumState)>,
    next_filter_id: u64,
    next_callout_id: u32,
    next_context_id: u64,
    auto_weight: u64,
}

pub const SHARED_ARENA: usize = 1 << 20;
pub const FIRST_HANDLE: u64 = 0x5746_5000;

#[repr(C)]
struct Header {
    lock: AtomicU32,
    len: AtomicU32,
    next: AtomicU64,
}

pub fn shared_mapping_name() -> String {
    format!("Local\\TomBox.NetView.Wfp.{}", unsafe { GetCurrentProcessId() })
}

fn shared() -> Option<usize> {
    static P: OnceLock<usize> = OnceLock::new();
    let p = *P.get_or_init(|| unsafe {
        let name: Vec<u16> = shared_mapping_name().encode_utf16().chain(Some(0)).collect();
        let size = (core::mem::size_of::<Header>() + SHARED_ARENA) as u32;
        let h = CreateFileMappingW(-1isize as *mut c_void, core::ptr::null_mut(), 0x04, 0, size, name.as_ptr());
        if h.is_null() {
            return 0;
        }
        MapViewOfFile(h, 0x0006, 0, 0, size as usize) as usize
    });
    (p != 0).then_some(p)
}

pub const AUTO_WEIGHT_TOP: u64 = (1u64 << 60) - 1;

impl Store {
    fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        self.sessions.put(&mut w);
        self.objects.put(&mut w);
        self.enums.put(&mut w);
        self.next_filter_id.put(&mut w);
        self.next_callout_id.put(&mut w);
        self.next_context_id.put(&mut w);
        self.auto_weight.put(&mut w);
        w.0
    }
    fn decode(next: u64, b: &[u8]) -> Option<Store> {
        if b.is_empty() {
            return Some(Store { next, sessions: Vec::new(), objects: Vec::new(), enums: Vec::new(), next_filter_id: 1, next_callout_id: 1, next_context_id: 1, auto_weight: AUTO_WEIGHT_TOP });
        }
        let mut r = R(b, 0);
        Some(Store {
            next,
            sessions: Codec::get(&mut r)?,
            objects: Codec::get(&mut r)?,
            enums: Codec::get(&mut r)?,
            next_filter_id: Codec::get(&mut r)?,
            next_callout_id: Codec::get(&mut r)?,
            next_context_id: Codec::get(&mut r)?,
            auto_weight: Codec::get(&mut r)?,
        })
    }
    fn alloc(&mut self) -> u64 {
        self.next += 4;
        self.next
    }
    fn session(&mut self, h: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.handle == h)
    }
    fn find(&self, kind: Kind, key: &[u8; 16]) -> Option<&Object> {
        self.objects.iter().map(|o| &o.object).find(|o| o.kind() == kind && o.key().as_ref() == Some(key))
    }
    fn has(&self, kind: Kind, key: &[u8; 16]) -> bool {
        self.find(kind, key).is_some()
    }
    fn add(&mut self, engine: u64, persistent: bool, o: Object) -> Result<(), u32> {
        let Some(sess) = self.session(engine) else {
            return Err(ERROR_INVALID_HANDLE);
        };
        let dynamic = sess.flags & FWPM_SESSION_FLAG_DYNAMIC != 0;
        if dynamic && persistent {
            return Err(FWP_E_DYNAMIC_SESSION_IN_PROGRESS);
        }
        if let Some(k) = o.key() {
            if self.has(o.kind(), &k) {
                return Err(FWP_E_ALREADY_EXISTS);
            }
        }
        if let Some(p) = o.provider_key() {
            if !self.has(Kind::Provider, &p) {
                return Err(FWP_E_PROVIDER_NOT_FOUND);
            }
        }
        self.objects.push(Stored { object: o, owner: if dynamic { Some(engine) } else { None } });
        Ok(())
    }
    fn referenced(&self, kind: Kind, key: &[u8; 16]) -> bool {
        self.objects.iter().any(|s| match (&s.object, kind) {
            (o, Kind::Provider) => o.provider_key().as_ref() == Some(key),
            (Object::Filter(f), Kind::SubLayer) => &f.sublayer == key,
            (Object::Filter(f), Kind::Callout) => f.action_type & FWP_ACTION_FLAG_CALLOUT != 0 && &f.action_key == key,
            (Object::Filter(f), Kind::ProviderContext) => f.flags & FWPM_FILTER_FLAG_HAS_PROVIDER_CONTEXT != 0 && &f.context == key,
            _ => false,
        })
    }
    fn delete(&mut self, engine: u64, kind: Kind, key: &[u8; 16], missing: u32) -> u32 {
        if self.session(engine).is_none() {
            return ERROR_INVALID_HANDLE;
        }
        let Some(i) = self.objects.iter().position(|s| s.object.kind() == kind && s.object.key().as_ref() == Some(key)) else {
            return missing;
        };
        if self.referenced(kind, key) {
            return FWP_E_IN_USE;
        }
        self.objects.remove(i);
        0
    }
    fn get(&mut self, engine: u64, kind: Kind, key: &[u8; 16], missing: u32) -> Result<Object, u32> {
        if self.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        self.find(kind, key).cloned().ok_or(missing)
    }
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> Result<T, u32>) -> Result<T, u32> {
    let base = shared().ok_or(ERROR_NOT_ENOUGH_MEMORY)?;
    let hdr = unsafe { &*(base as *const Header) };
    while hdr.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::thread::yield_now();
    }
    let arena = unsafe { core::slice::from_raw_parts_mut((base + core::mem::size_of::<Header>()) as *mut u8, SHARED_ARENA) };
    let len = (hdr.len.load(Ordering::Relaxed) as usize).min(SHARED_ARENA);
    let next = match hdr.next.load(Ordering::Relaxed) {
        0 => FIRST_HANDLE,
        n => n,
    };
    let out = match Store::decode(next, &arena[..len]) {
        None => Err(ERROR_NOT_ENOUGH_MEMORY),
        Some(mut st) => match f(&mut st) {
            Err(e) => Err(e),
            Ok(v) => {
                let bytes = st.encode();
                if bytes.len() > SHARED_ARENA {
                    Err(ERROR_NOT_ENOUGH_MEMORY)
                } else {
                    arena[..bytes.len()].copy_from_slice(&bytes);
                    hdr.len.store(bytes.len() as u32, Ordering::Relaxed);
                    hdr.next.store(st.next, Ordering::Relaxed);
                    Ok(v)
                }
            }
        },
    };
    hdr.lock.store(0, Ordering::Release);
    out
}

fn status(r: Result<u32, u32>) -> u32 {
    match r {
        Ok(v) | Err(v) => v,
    }
}

fn code(r: Result<(), u32>) -> u32 {
    match r {
        Ok(()) => 0,
        Err(e) => e,
    }
}

pub fn is_builtin(prefix: &str, key: &[u8; 16]) -> bool {
    wfp_names::NAMES.iter().any(|(k, n)| k == key && n.starts_with(prefix))
}

pub fn engine_open(session_flags: u32) -> Result<u64, u32> {
    with_store(|s| {
        let h = s.alloc();
        s.sessions.push(Session { handle: h, flags: session_flags, txn: false });
        Ok(h)
    })
}

pub fn engine_close(engine: u64) -> u32 {
    status(with_store(|s| {
        let Some(i) = s.sessions.iter().position(|x| x.handle == engine) else {
            return Ok(ERROR_INVALID_HANDLE);
        };
        let sess = s.sessions.remove(i);
        if sess.flags & FWPM_SESSION_FLAG_DYNAMIC != 0 {
            s.objects.retain(|p| p.owner != Some(engine));
        }
        s.enums.retain(|(_, e)| e.engine != engine);
        Ok(0)
    }))
}

pub fn transaction_begin(engine: u64) -> u32 {
    status(with_store(|s| {
        Ok(match s.session(engine) {
            None => ERROR_INVALID_HANDLE,
            Some(x) if x.txn => FWP_E_TXN_IN_PROGRESS,
            Some(x) => {
                x.txn = true;
                0
            }
        })
    }))
}

pub fn transaction_commit(engine: u64) -> u32 {
    status(with_store(|s| {
        Ok(match s.session(engine) {
            None => ERROR_INVALID_HANDLE,
            Some(x) if !x.txn => FWP_E_NO_TXN_IN_PROGRESS,
            Some(x) => {
                x.txn = false;
                0
            }
        })
    }))
}

pub fn transaction_abort(engine: u64) -> u32 {
    transaction_commit(engine)
}

pub fn provider_add(engine: u64, p: Provider) -> u32 {
    status(with_store(|s| {
        if s.session(engine).is_none() {
            return Ok(ERROR_INVALID_HANDLE);
        }
        if p.name.is_none() {
            return Ok(ERROR_INVALID_PARAMETER);
        }
        let persistent = p.flags & FWPM_PROVIDER_FLAG_PERSISTENT != 0;
        Ok(code(s.add(engine, persistent, Object::Provider(p))))
    }))
}

pub fn provider_get_by_key(engine: u64, key: &[u8; 16]) -> Result<Provider, u32> {
    match with_store(|s| s.get(engine, Kind::Provider, key, FWP_E_PROVIDER_NOT_FOUND))? {
        Object::Provider(p) => Ok(p),
        _ => Err(FWP_E_PROVIDER_NOT_FOUND),
    }
}

pub fn provider_delete_by_key(engine: u64, key: &[u8; 16]) -> u32 {
    status(with_store(|s| Ok(s.delete(engine, Kind::Provider, key, FWP_E_PROVIDER_NOT_FOUND))))
}

pub fn sublayer_add(engine: u64, l: SubLayer) -> u32 {
    status(with_store(|s| {
        if s.session(engine).is_none() {
            return Ok(ERROR_INVALID_HANDLE);
        }
        if l.name.is_none() {
            return Ok(ERROR_INVALID_PARAMETER);
        }
        if is_builtin("FWPM_SUBLAYER_", &l.key) {
            return Ok(FWP_E_ALREADY_EXISTS);
        }
        let persistent = l.flags & FWPM_SUBLAYER_FLAG_PERSISTENT != 0;
        Ok(code(s.add(engine, persistent, Object::SubLayer(l))))
    }))
}

pub fn sublayer_get_by_key(engine: u64, key: &[u8; 16]) -> Result<SubLayer, u32> {
    match with_store(|s| s.get(engine, Kind::SubLayer, key, FWP_E_SUBLAYER_NOT_FOUND))? {
        Object::SubLayer(v) => Ok(v),
        _ => Err(FWP_E_SUBLAYER_NOT_FOUND),
    }
}

pub fn sublayer_delete_by_key(engine: u64, key: &[u8; 16]) -> u32 {
    status(with_store(|s| Ok(s.delete(engine, Kind::SubLayer, key, FWP_E_SUBLAYER_NOT_FOUND))))
}

pub fn callout_add(engine: u64, mut c: Callout) -> Result<u32, u32> {
    with_store(|s| {
        if s.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        if c.name.is_none() {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if !is_builtin("FWPM_LAYER_", &c.layer) {
            return Err(FWP_E_LAYER_NOT_FOUND);
        }
        if is_builtin("FWPM_CALLOUT_", &c.key) {
            return Err(FWP_E_ALREADY_EXISTS);
        }
        let id = s.next_callout_id;
        c.id = id;
        let persistent = c.flags & FWPM_CALLOUT_FLAG_PERSISTENT != 0;
        s.add(engine, persistent, Object::Callout(c))?;
        s.next_callout_id += 1;
        Ok(id)
    })
}

pub fn callout_get_by_key(engine: u64, key: &[u8; 16]) -> Result<Callout, u32> {
    match with_store(|s| s.get(engine, Kind::Callout, key, FWP_E_CALLOUT_NOT_FOUND))? {
        Object::Callout(v) => Ok(v),
        _ => Err(FWP_E_CALLOUT_NOT_FOUND),
    }
}

pub fn callout_delete_by_key(engine: u64, key: &[u8; 16]) -> u32 {
    status(with_store(|s| Ok(s.delete(engine, Kind::Callout, key, FWP_E_CALLOUT_NOT_FOUND))))
}

fn effective_weight(w: &Value, auto: &mut u64) -> Result<Value, u32> {
    match w {
        Value::U64(v) => Ok(Value::U64(*v)),
        Value::U8(v) if *v <= 15 => Ok(Value::U64((*v as u64) << 60 | (*auto & AUTO_WEIGHT_TOP))),
        Value::Empty => Ok(Value::U64(*auto)),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

pub fn filter_add(engine: u64, mut f: Filter) -> Result<u64, u32> {
    with_store(|s| {
        if s.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        if f.name.is_none() {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if !is_builtin("FWPM_LAYER_", &f.layer) {
            return Err(FWP_E_LAYER_NOT_FOUND);
        }
        if f.sublayer != [0u8; 16] && !s.has(Kind::SubLayer, &f.sublayer) && !is_builtin("FWPM_SUBLAYER_", &f.sublayer) {
            return Err(FWP_E_SUBLAYER_NOT_FOUND);
        }
        if f.action_type & FWP_ACTION_FLAG_CALLOUT != 0 && !s.has(Kind::Callout, &f.action_key) && !is_builtin("FWPM_CALLOUT_", &f.action_key) {
            return Err(FWP_E_CALLOUT_NOT_FOUND);
        }
        if f.flags & FWPM_FILTER_FLAG_HAS_PROVIDER_CONTEXT != 0 && !s.has(Kind::ProviderContext, &f.context) {
            return Err(FWP_E_PROVIDER_CONTEXT_NOT_FOUND);
        }
        let mut auto = s.auto_weight;
        f.effective_weight = effective_weight(&f.weight, &mut auto)?;
        let id = s.next_filter_id;
        f.id = id;
        let persistent = f.flags & FWPM_FILTER_FLAG_PERSISTENT != 0;
        s.add(engine, persistent, Object::Filter(f))?;
        s.next_filter_id += 1;
        s.auto_weight = s.auto_weight.saturating_sub(1);
        Ok(id)
    })
}

pub fn filter_get_by_key(engine: u64, key: &[u8; 16]) -> Result<Filter, u32> {
    match with_store(|s| s.get(engine, Kind::Filter, key, FWP_E_FILTER_NOT_FOUND))? {
        Object::Filter(v) => Ok(v),
        _ => Err(FWP_E_FILTER_NOT_FOUND),
    }
}

pub fn filter_delete_by_key(engine: u64, key: &[u8; 16]) -> u32 {
    status(with_store(|s| Ok(s.delete(engine, Kind::Filter, key, FWP_E_FILTER_NOT_FOUND))))
}

pub fn filter_delete_by_id(engine: u64, id: u64) -> u32 {
    status(with_store(|s| {
        if s.session(engine).is_none() {
            return Ok(ERROR_INVALID_HANDLE);
        }
        Ok(match s.objects.iter().position(|o| matches!(&o.object, Object::Filter(f) if f.id == id)) {
            Some(i) => {
                s.objects.remove(i);
                0
            }
            None => FWP_E_FILTER_NOT_FOUND,
        })
    }))
}

fn body_fits(kind: u32, body: &ContextBody) -> bool {
    match (kind, body) {
        (FWPM_IPSEC_KEYING_CONTEXT, ContextBody::Keying { .. }) => true,
        (FWPM_IPSEC_KEYING_CONTEXT, _) => false,
        (1 | 3 | 12, ContextBody::Transport(_)) => true,
        (5 | 6 | 10, ContextBody::MainMode(_)) => true,
        (FWPM_CLASSIFY_OPTIONS_CONTEXT, ContextBody::ClassifyOptions(_)) => true,
        (FWPM_GENERAL_CONTEXT, ContextBody::General(_)) => true,
        (2 | 4 | 9 | 11, ContextBody::Opaque) => true,
        _ => false,
    }
}

pub fn provider_context_add(engine: u64, mut c: ProviderContext) -> Result<u64, u32> {
    with_store(|s| {
        if s.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        if c.name.is_none() || c.kind >= FWPM_PROVIDER_CONTEXT_TYPE_MAX || !body_fits(c.kind, &c.body) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let id = s.next_context_id;
        c.id = id;
        let persistent = c.flags & FWPM_PROVIDER_CONTEXT_FLAG_PERSISTENT != 0;
        s.add(engine, persistent, Object::ProviderContext(c))?;
        s.next_context_id += 1;
        Ok(id)
    })
}

pub fn provider_context_get_by_key(engine: u64, key: &[u8; 16]) -> Result<ProviderContext, u32> {
    match with_store(|s| s.get(engine, Kind::ProviderContext, key, FWP_E_PROVIDER_CONTEXT_NOT_FOUND))? {
        Object::ProviderContext(v) => Ok(v),
        _ => Err(FWP_E_PROVIDER_CONTEXT_NOT_FOUND),
    }
}

pub fn provider_context_delete_by_key(engine: u64, key: &[u8; 16]) -> u32 {
    status(with_store(|s| Ok(s.delete(engine, Kind::ProviderContext, key, FWP_E_PROVIDER_CONTEXT_NOT_FOUND))))
}

fn open_enum(s: &mut Store, engine: u64, items: Vec<Object>) -> u64 {
    let h = s.alloc();
    s.enums.push((h, EnumState { engine, items, pos: 0 }));
    h
}

pub fn create_enum_handle(engine: u64, kind: Kind) -> Result<u64, u32> {
    with_store(|s| {
        if s.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        let items = s.objects.iter().map(|o| o.object.clone()).filter(|o| o.kind() == kind).collect();
        Ok(open_enum(s, engine, items))
    })
}

pub fn create_filter_enum_handle(engine: u64, layer: Option<[u8; 16]>) -> Result<u64, u32> {
    with_store(|s| {
        if s.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        if let Some(l) = layer {
            if !is_builtin("FWPM_LAYER_", &l) {
                return Err(FWP_E_LAYER_NOT_FOUND);
            }
        }
        let items = s.objects.iter().filter_map(|o| match &o.object {
            Object::Filter(f) if layer.map(|l| l == f.layer).unwrap_or(true) => Some(o.object.clone()),
            _ => None,
        }).collect();
        Ok(open_enum(s, engine, items))
    })
}

pub fn enum_next(engine: u64, handle: u64, requested: u32) -> Result<Vec<Object>, u32> {
    with_store(|s| {
        if s.session(engine).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        let Some((_, e)) = s.enums.iter_mut().find(|(h, e)| *h == handle && e.engine == engine) else {
            return Err(ERROR_INVALID_HANDLE);
        };
        let pos = e.pos as usize;
        let end = (pos + requested as usize).min(e.items.len());
        let out = e.items[pos..end].to_vec();
        e.pos = end as u32;
        Ok(out)
    })
}

pub fn destroy_enum_handle(engine: u64, handle: u64) -> u32 {
    status(with_store(|s| {
        if s.session(engine).is_none() {
            return Ok(ERROR_INVALID_HANDLE);
        }
        Ok(match s.enums.iter().position(|(h, e)| *h == handle && e.engine == engine) {
            Some(i) => {
                s.enums.remove(i);
                0
            }
            None => ERROR_INVALID_HANDLE,
        })
    }))
}

pub fn provider_count() -> usize {
    with_store(|s| Ok(s.objects.iter().filter(|o| o.object.kind() == Kind::Provider).count())).unwrap_or(0)
}

pub fn object_count(kind: Kind) -> usize {
    with_store(|s| Ok(s.objects.iter().filter(|o| o.object.kind() == kind).count())).unwrap_or(0)
}

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
