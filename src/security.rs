use crate::fs::VfsNode;
use crate::locks::Spinlock;
use crate::sched::Credentials;
use pbkdf2::pbkdf2_hmac;
use sha2::Digest;
use sha2::Sha256;

const PASSWORD_HASH_SIZE: usize = 32;
const MAX_ACCOUNTS: usize = 16;
const USERNAME_SIZE: usize = 32;
const DUMMY_PASSWORD_ROUNDS: u32 = 100_000;
const MIN_ENTROPY_SAMPLES: u32 = 32;

struct EntropyPool {
    state: [u32; 8],
    samples: u32,
}

static ENTROPY_POOL: Spinlock<EntropyPool> = Spinlock::new(EntropyPool {
    state: [
        0x243f_6a88,
        0x85a3_08d3,
        0x1319_8a2e,
        0x0370_7344,
        0xa409_3822,
        0x299f_31d0,
        0x082e_fa98,
        0xec4e_6c89,
    ],
    samples: 0,
});

pub(crate) fn mix_entropy() {
    let timestamp = unsafe { read_tsc() };
    let mut pool = ENTROPY_POOL.lock();
    let sample = timestamp as u32 ^ (timestamp >> 32) as u32;
    let index = (pool.samples as usize) % pool.state.len();
    pool.state[index] =
        pool.state[index].rotate_left(7) ^ sample ^ pool.samples.wrapping_mul(0x9e37_79b9);
    let next = (index + 1) % pool.state.len();
    pool.state[next] = pool.state[next].wrapping_add(sample.rotate_left(13));
    pool.samples = pool.samples.saturating_add(1);
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PasswordRecord {
    pub uid: u32,
    pub gid: u32,
    pub salt: [u8; 16],
    pub hash: [u8; PASSWORD_HASH_SIZE],
    pub rounds: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AccountRecord {
    pub username: [u8; USERNAME_SIZE],
    pub password: PasswordRecord,
}

pub(crate) static ACCOUNT_TABLE: Spinlock<[Option<AccountRecord>; MAX_ACCOUNTS]> = {
    const EMPTY: Option<AccountRecord> = None;
    Spinlock::new([EMPTY; MAX_ACCOUNTS])
};

pub(crate) fn has_accounts() -> bool {
    ACCOUNT_TABLE.lock().iter().any(Option::is_some)
}

pub(crate) fn has_root_account() -> bool {
    let accounts = ACCOUNT_TABLE.lock();
    accounts.iter().flatten().any(|account| {
        account.password.uid == 0
            || constant_time_equal(b"root", username_bytes(&account.username))
    })
}

pub(crate) fn get_account_uid(username: &[u8]) -> Option<u32> {
    if constant_time_equal(username, b"root") {
        return Some(0);
    }
    let accounts = ACCOUNT_TABLE.lock();
    accounts
        .iter()
        .flatten()
        .find(|account| constant_time_equal(username, username_bytes(&account.username)))
        .map(|account| account.password.uid)
}

pub(crate) fn username_for_uid(uid: u32, output: &mut [u8]) -> Option<usize> {
    let accounts = ACCOUNT_TABLE.lock();
    if let Some(account) = accounts
        .iter()
        .flatten()
        .find(|account| account.password.uid == uid)
    {
        let name = username_bytes(&account.username);
        if name.len() > output.len() {
            return None;
        }
        output[..name.len()].copy_from_slice(name);
        return Some(name.len());
    }
    if uid == 0 {
        if output.len() < 4 {
            return None;
        }
        output[..4].copy_from_slice(b"root");
        return Some(4);
    }
    None
}

pub(crate) fn credentials_for_user(username: &[u8]) -> Option<Credentials> {
    if constant_time_equal(username, b"root") {
        let accounts = ACCOUNT_TABLE.lock();
        let root_account = accounts
            .iter()
            .flatten()
            .find(|account| constant_time_equal(username, username_bytes(&account.username)))
            .copied();
        drop(accounts);
        if root_account.is_none() {
            return Some(Credentials::root());
        }
    }

    let accounts = ACCOUNT_TABLE.lock();
    let account = accounts
        .iter()
        .flatten()
        .find(|account| constant_time_equal(username, username_bytes(&account.username)))
        .copied()?;
    drop(accounts);

    let uid = account.password.uid;
    let gid = account.password.gid;
    Some(Credentials {
        uid,
        gid,
        euid: uid,
        egid: gid,
        fsuid: uid,
        fsgid: gid,
        groups: [0; 8],
        group_count: 0,
    })
}

pub(crate) fn ensure_root_account() -> bool {
    if has_root_account() {
        return true;
    }
    let Some(account) = create_account_record(b"root", 0, 0, b"root", 100_000) else {
        return false;
    };
    install_or_update_account(account)
}

pub(crate) fn next_user_id() -> Option<u32> {
    let accounts = ACCOUNT_TABLE.lock();
    accounts
        .iter()
        .flatten()
        .map(|account| account.password.uid)
        .max()
        .unwrap_or(999)
        .checked_add(1)
}

pub(crate) fn verify_password(record: &PasswordRecord, password: &[u8]) -> bool {
    if record.rounds == 0 {
        return false;
    }

    let mut derived = [0u8; PASSWORD_HASH_SIZE];
    pbkdf2_hmac::<Sha256>(password, &record.salt, record.rounds, &mut derived);
    constant_time_equal(&derived, &record.hash)
}

pub(crate) fn authenticate(username: &[u8], password: &[u8]) -> Option<Credentials> {
    let accounts = ACCOUNT_TABLE.lock();
    let account = accounts
        .iter()
        .flatten()
        .find(|account| constant_time_equal(username, username_bytes(&account.username)))
        .copied();
    drop(accounts);

    let password_record = account
        .map(|account| account.password)
        .unwrap_or(PasswordRecord {
            uid: 0,
            gid: 0,
            salt: [0; 16],
            hash: [0; PASSWORD_HASH_SIZE],
            rounds: DUMMY_PASSWORD_ROUNDS,
        });
    let verified = verify_password(&password_record, password);
    let uname_str = core::str::from_utf8(username).unwrap_or("<utf8 err>");
    crate::pr_info!(
        "[AUTH] Authenticate user '{}' (len={}): found={}, verified={}\n",
        uname_str,
        username.len(),
        account.is_some(),
        verified
    );
    if !verified || account.is_none() {
        return None;
    }

    let uid = password_record.uid;
    let gid = password_record.gid;
    return Some(Credentials {
        uid,
        gid,
        euid: uid,
        egid: gid,
        fsuid: uid,
        fsgid: gid,
        groups: [0; 8],
        group_count: 0,
    });
}

pub(crate) fn create_password_record(
    uid: u32,
    gid: u32,
    password: &[u8],
    rounds: u32,
) -> Option<PasswordRecord> {
    if rounds == 0 {
        return None;
    }

    let mut salt = [0u8; 16];
    fill_random(&mut salt)?;
    let mut hash = [0u8; PASSWORD_HASH_SIZE];
    pbkdf2_hmac::<Sha256>(password, &salt, rounds, &mut hash);
    Some(PasswordRecord { uid, gid, salt, hash, rounds })
}

pub(crate) fn create_account_record(
    username: &[u8],
    uid: u32,
    gid: u32,
    password: &[u8],
    rounds: u32,
) -> Option<AccountRecord> {
    if username.is_empty() || username.len() >= USERNAME_SIZE {
        return None;
    }

    let mut name = [0u8; USERNAME_SIZE];
    name[..username.len()].copy_from_slice(username);
    Some(AccountRecord {
        username: name,
        password: create_password_record(uid, gid, password, rounds)?,
    })
}

pub(crate) fn load_accounts(data: &[u8]) -> usize {
    let mut loaded = 0;
    for line in data.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }

        let mut fields = line.split(|&byte| byte == b':');
        let Some(username) = fields.next() else {
            continue;
        };
        let Some(uid) = fields.next().and_then(parse_decimal) else {
            continue;
        };
        let Some(gid) = fields.next().and_then(parse_decimal) else {
            continue;
        };
        let Some(rounds) = fields.next().and_then(parse_decimal) else {
            continue;
        };
        let Some(salt_field) = fields.next() else {
            continue;
        };
        let Some(hash_field) = fields.next() else {
            continue;
        };
        if fields.next().is_some() {
            continue;
        }

        let Some(salt) = parse_hex::<16>(salt_field) else {
            continue;
        };
        let Some(hash) = parse_hex::<PASSWORD_HASH_SIZE>(hash_field) else {
            continue;
        };
        let Some(account) = create_account_from_hash(username, uid, gid, salt, hash, rounds) else {
            continue;
        };
        if install_account(account) {
            loaded += 1;
        }
    }
    loaded
}

fn create_account_from_hash(
    username: &[u8],
    uid: u32,
    gid: u32,
    salt: [u8; 16],
    hash: [u8; PASSWORD_HASH_SIZE],
    rounds: u32,
) -> Option<AccountRecord> {
    if username.is_empty() || username.len() >= USERNAME_SIZE || rounds == 0 {
        return None;
    }

    let mut name = [0u8; USERNAME_SIZE];
    name[..username.len()].copy_from_slice(username);
    Some(AccountRecord {
        username: name,
        password: PasswordRecord { uid, gid, salt, hash, rounds },
    })
}

fn parse_decimal(value: &[u8]) -> Option<u32> {
    if value.is_empty() {
        return None;
    }

    let mut result = 0u32;
    for &byte in value {
        if !byte.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
    }
    Some(result)
}

fn parse_hex<const SIZE: usize>(value: &[u8]) -> Option<[u8; SIZE]> {
    if value.len() != SIZE * 2 {
        return None;
    }

    let mut result = [0u8; SIZE];
    for (index, pair) in value.chunks_exact(2).enumerate() {
        result[index] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Some(result)
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn fill_random(output: &mut [u8]) -> Option<()> {
    if rdrand_supported() {
        if let Some(()) = fill_rdrand(output) {
            return Some(());
        }
    }

    let mut pool = ENTROPY_POOL.lock();
    while pool.samples < MIN_ENTROPY_SAMPLES {
        let timestamp = unsafe { read_tsc() };
        let sample = timestamp as u32 ^ (timestamp >> 32) as u32;
        let index = (pool.samples as usize) % pool.state.len();
        pool.state[index] =
            pool.state[index].rotate_left(7) ^ sample ^ pool.samples.wrapping_mul(0x9e37_79b9);
        let next = (index + 1) % pool.state.len();
        pool.state[next] = pool.state[next].wrapping_add(sample.rotate_left(13));
        pool.samples = pool.samples.saturating_add(1);
    }

    let mut digest = Sha256::new();
    for word in pool.state {
        digest.update(word.to_le_bytes());
    }
    digest.update(pool.samples.to_le_bytes());
    let digest = digest.finalize();
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = digest[index % digest.len()];
    }
    pool.state[0] ^= u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    pool.samples = 0;
    Some(())
}

fn fill_rdrand(output: &mut [u8]) -> Option<()> {
    for chunk in output.chunks_mut(4) {
        let mut value: u32;
        let mut ready: u8;
        let mut attempts = 0;
        loop {
            unsafe {
                core::arch::asm!(
                    "rdrand {value:e}",
                    "setc {ready}",
                    value = out(reg) value,
                    ready = out(reg_byte) ready,
                    options(nostack, nomem, preserves_flags),
                );
            }
            if ready != 0 || attempts == 10 {
                break;
            }
            attempts += 1;
        }
        if ready == 0 {
            return None;
        }
        let bytes = value.to_le_bytes();
        let length = chunk.len();
        chunk.copy_from_slice(&bytes[..length]);
    }
    Some(())
}

unsafe fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdtsc",
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags),
    );
    ((high as u64) << 32) | low as u64
}

fn rdrand_supported() -> bool {
    let features = unsafe { core::arch::x86::__cpuid(1) };
    features.ecx & (1 << 30) != 0
}

pub(crate) fn install_account(account: AccountRecord) -> bool {
    let mut accounts = ACCOUNT_TABLE.lock();
    if accounts.iter().flatten().any(|existing| {
        constant_time_equal(
            username_bytes(&existing.username),
            username_bytes(&account.username),
        )
    }) {
        return false;
    }

    if let Some(slot) = accounts.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(account);
        true
    } else {
        false
    }
}

pub(crate) fn install_or_update_account(account: AccountRecord) -> bool {
    let mut accounts = ACCOUNT_TABLE.lock();
    if let Some(existing) = accounts.iter_mut().flatten().find(|existing| {
        constant_time_equal(
            username_bytes(&existing.username),
            username_bytes(&account.username),
        )
    }) {
        *existing = account;
        return true;
    }

    if let Some(slot) = accounts.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(account);
        true
    } else {
        false
    }
}

pub(crate) fn serialize_accounts(output: &mut [u8]) -> Option<usize> {
    let accounts = ACCOUNT_TABLE.lock();
    let mut written: usize = 0;
    for account in accounts.iter().flatten() {
        let mut line = [0u8; 256];
        let mut length = 0;
        length = append_bytes(&mut line, length, username_bytes(&account.username))?;
        length = append_byte(&mut line, length, b':')?;
        length = append_decimal(&mut line, length, account.password.uid)?;
        length = append_byte(&mut line, length, b':')?;
        length = append_decimal(&mut line, length, account.password.gid)?;
        length = append_byte(&mut line, length, b':')?;
        length = append_decimal(&mut line, length, account.password.rounds)?;
        length = append_byte(&mut line, length, b':')?;
        length = append_hex(&mut line, length, &account.password.salt)?;
        length = append_byte(&mut line, length, b':')?;
        length = append_hex(&mut line, length, &account.password.hash)?;
        length = append_byte(&mut line, length, b'\n')?;
        if written.checked_add(length)? > output.len() {
            return None;
        }
        output[written..written + length].copy_from_slice(&line[..length]);
        written += length;
    }
    Some(written)
}

pub(crate) unsafe fn persist_accounts() -> bool {
    let root = crate::fs::ROOT_NODE;
    let etc = match crate::fs::resolve_path("/etc", root) {
        Ok(node) => node,
        Err(_) => match crate::fs::create_child_node(
            root,
            "etc",
            crate::fs::VfsNodeType::Directory,
            0o755,
        ) {
            Ok(node) => node,
            Err(_) => return false,
        },
    };
    let shadow = match crate::fs::resolve_path("/etc/shadow", root) {
        Ok(node) => node,
        Err(_) => {
            match crate::fs::create_child_node(etc, "shadow", crate::fs::VfsNodeType::File, 0o600) {
                Ok(node) => node,
                Err(_) => return false,
            }
        }
    };
    if (*shadow).inode == 0 || (*shadow).node_type != crate::fs::VfsNodeType::File {
        return false;
    }
    if (*shadow).truncate().is_err() {
        return false;
    }
    let data_alloc = match crate::paging::kmalloc(4096) {
        Ok(ptr) => ptr,
        Err(_) => return false,
    };
    let data = &mut *(data_alloc as *mut [u8; 4096]);
    data.fill(0);
    let length_opt = serialize_accounts(data);
    let result = if let Some(length) = length_opt {
        (*shadow)
            .write(&data[..length], 0)
            .map(|written| written == length)
            .unwrap_or(false)
    } else {
        false
    };
    let _ = crate::paging::kfree(data_alloc);
    result
}

fn append_bytes(output: &mut [u8], offset: usize, value: &[u8]) -> Option<usize> {
    let end = offset.checked_add(value.len())?;
    output.get_mut(offset..end)?.copy_from_slice(value);
    Some(end)
}

fn append_byte(output: &mut [u8], offset: usize, value: u8) -> Option<usize> {
    *output.get_mut(offset)? = value;
    Some(offset + 1)
}

fn append_decimal(output: &mut [u8], mut offset: usize, mut value: u32) -> Option<usize> {
    let start = offset;
    let mut digits = [0u8; 10];
    let mut count = 0;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        offset = append_byte(output, offset, digits[count])?;
    }
    if offset == start {
        None
    } else {
        Some(offset)
    }
}

fn append_hex<const SIZE: usize>(
    output: &mut [u8],
    mut offset: usize,
    value: &[u8; SIZE],
) -> Option<usize> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &byte in value {
        offset = append_byte(output, offset, HEX[(byte >> 4) as usize])?;
        offset = append_byte(output, offset, HEX[(byte & 0x0f) as usize])?;
    }
    Some(offset)
}

pub(crate) unsafe fn set_current_credentials(credentials: Credentials) {
    let task = crate::sched::current();
    if let Some(t) = task.as_mut() {
        t.credentials = credentials;
    } else {
        let mut table = crate::sched::PROCESS_TABLE.lock();
        if let Some(ref mut idle) = table[0] {
            idle.credentials = credentials;
        }
    }
}

pub(crate) unsafe fn login_current(username: &[u8], password: &[u8]) -> bool {
    let Some(credentials) = authenticate(username, password) else {
        return false;
    };

    set_current_credentials(credentials);
    true
}

fn username_bytes(username: &[u8; USERNAME_SIZE]) -> &[u8] {
    let length = username
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(USERNAME_SIZE);
    &username[..length]
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut difference = 0u8;
    for (&left_byte, &right_byte) in left.iter().zip(right.iter()) {
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Read,
    ReadWrite,
    Write,
    Traverse,
    Create,
    Truncate,
    Mount,
    Unmount,
    AccountAdmin,
    Signal,
    Inspect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SecurityObject {
    File {
        owner_uid: u32,
        owner_gid: u32,
        mode: u16,
    },
    Process {
        owner_uid: u32,
    },
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Allow,
    Deny,
}

pub(crate) unsafe fn object_for_node(node: *const VfsNode) -> SecurityObject {
    SecurityObject::File {
        owner_uid: (*node).owner_uid,
        owner_gid: (*node).owner_gid,
        mode: (*node).rights,
    }
}

pub(crate) fn check(
    credentials: &Credentials,
    object: &SecurityObject,
    operation: Operation,
) -> Decision {
    let dac_allowed = match (object, operation) {
        (SecurityObject::File { owner_uid, owner_gid, mode }, Operation::Read) => {
            file_permission(credentials, *owner_uid, *owner_gid, *mode, 0o4)
        }
        (SecurityObject::File { owner_uid, owner_gid, mode }, Operation::ReadWrite) => {
            file_permission(credentials, *owner_uid, *owner_gid, *mode, 0o6)
        }
        (SecurityObject::File { owner_uid, owner_gid, mode }, Operation::Write) => {
            file_permission(credentials, *owner_uid, *owner_gid, *mode, 0o2)
        }
        (SecurityObject::File { owner_uid, owner_gid, mode }, Operation::Traverse) => {
            file_permission(credentials, *owner_uid, *owner_gid, *mode, 0o1)
        }
        (SecurityObject::File { owner_uid, owner_gid, mode }, Operation::Create) => {
            file_permission(credentials, *owner_uid, *owner_gid, *mode, 0o3)
        }
        (SecurityObject::File { owner_uid, owner_gid, mode }, Operation::Truncate) => {
            file_permission(credentials, *owner_uid, *owner_gid, *mode, 0o2)
        }
        (SecurityObject::Process { owner_uid }, Operation::Signal)
        | (SecurityObject::Process { owner_uid }, Operation::Inspect) => {
            credentials.euid == *owner_uid
        }
        (SecurityObject::System, Operation::Mount)
        | (SecurityObject::System, Operation::Unmount)
        | (SecurityObject::System, Operation::AccountAdmin) => false,
        _ => false,
    };

    if dac_allowed {
        Decision::Allow
    } else if credentials.is_root() {
        Decision::Allow
    } else {
        Decision::Deny
    }
}

fn file_permission(
    credentials: &Credentials,
    owner_uid: u32,
    owner_gid: u32,
    mode: u16,
    requested: u16,
) -> bool {
    let class_bits = if credentials.fsuid == owner_uid {
        (mode >> 6) & 0o7
    } else if credentials.in_group(owner_gid) {
        (mode >> 3) & 0o7
    } else {
        mode & 0o7
    };

    class_bits & requested == requested
}
