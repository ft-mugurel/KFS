use crate::pr_warn;

#[expect(dead_code, reason = "full errno table; not every code is returned yet")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum KernelError {
    /** Operation not permitted */
    Perm = -1,
    /** No such file or directory */
    NoEnt = -2,
    /** No such process */
    Srch = -3,
    /** Interrupted system call */
    Intr = -4,
    /** Input/output error */
    IO = -5,
    /** No such device or address */
    NxIO = -6,
    /** Argument list too long */
    TooBig = -7,
    /** Exec format error */
    NoExec = -8,
    /** Bad file descriptor */
    BadF = -9,
    /** No child processes */
    Child = -10,
    /** Resource temporarily unavailable */
    Again = -11,
    /** Cannot allocate memory */
    NoMem = -12,
    /** Permission denied */
    Acces = -13,
    /** Bad address */
    Fault = -14,
    /** Block device required */
    NotBlk = -15,
    /** Device or resource busy */
    Busy = -16,
    /** File exists */
    Exist = -17,
    /** Invalid cross-device link */
    XDev = -18,
    /** No such device */
    NoDev = -19,
    /** Not a directory */
    NotDir = -20,
    /** Is a directory */
    IsDir = -21,
    /** Invalid argument */
    Inval = -22,
    /** Too many open files in system */
    NFile = -23,
    /** Too many open files */
    MFile = -24,
    /** Inappropriate ioctl for device */
    NotTTY = -25,
    /** Text file busy */
    TxtBsy = -26,
    /** File too large */
    FBig = -27,
    /** No space left on device */
    NoSpc = -28,
    /** Illegal seek */
    SPipe = -29,
    /** Read-only file system */
    #[allow(clippy::upper_case_acronyms)]
    ROFS = -30,
    /** Too many links */
    MLink = -31,
    /** Broken pipe */
    Pipe = -32,
    /** Numerical argument out of domain */
    Dom = -33,
    /** Numerical result out of range */
    Range = -34,
    /** Resource deadlock avoided */
    Deadlock = -35,
    /** File name too long */
    NameTooLong = -36,
    /** No locks available */
    NoLck = -37,
    /** Function not implemented */
    NoSys = -38,
    /** Directory not empty */
    NotEmpty = -39,
    /** Too many levels of symbolic links */
    Loop = -40,
    /** No message of desired type */
    NoMsg = -42,
    /** Identifier removed */
    IdRm = -43,
    /** Channel number out of range */
    ChRng = -44,
    /** Level 2 not synchronised */
    L2NSync = -45,
    /** Level 3 halted */
    L3Hlt = -46,
    /** Level 3 reset */
    L3Rst = -47,
    /** Link number out of range */
    LnRng = -48,
    /** Protocol driver not attached */
    Unatch = -49,
    /** No CSI structure available */
    NoCSI = -50,
    /** Level 2 halted */
    L2Hlt = -51,
    /** Invalid exchange */
    BadE = -52,
    /** Invalid request descriptor */
    BadR = -53,
    /** Exchange full */
    XFull = -54,
    /** No anode */
    NoAno = -55,
    /** Invalid request code */
    BadRqC = -56,
    /** Invalid slot */
    BadSlt = -57,
    /** Bad font file format */
    BFont = -59,
    /** Device not a stream */
    NoStr = -60,
    /** No data available */
    NoData = -61,
    /** Timer expired */
    Time = -62,
    /** Out of streams resources */
    NoSR = -63,
    /** Machine is not on the network */
    NoNet = -64,
    /** Package not installed */
    NoPkg = -65,
    /** Object is remote */
    Remote = -66,
    /** Link has been severed */
    NoLink = -67,
    /** Advertise error */
    Adv = -68,
    /** Srmount error */
    SrMnt = -69,
    /** Communication error on send */
    Comm = -70,
    /** Protocol error */
    Proto = -71,
    /** Multihop attempted */
    MultiHop = -72,
    /** RFS specific error */
    DotDot = -73,
    /** Bad message */
    BadMsg = -74,
    /** Value too large for defined data type */
    Overflow = -75,
    /** Name not unique on network */
    NotUniq = -76,
    /** File descriptor in bad state */
    BadFD = -77,
    /** Remote address changed */
    RemChg = -78,
    /** Can not access a needed shared library */
    LibAcc = -79,
    /** Accessing a corrupted shared library */
    LibBad = -80,
    /** .lib section in a.out corrupted */
    LibScn = -81,
    /** Attempting to link in too many shared libraries */
    LibMax = -82,
    /** Cannot exec a shared library directly */
    LibExec = -83,
    /** Invalid or incomplete multibyte or wide character */
    IlSeq = -84,
    /** Interrupted system call should be restarted */
    Restart = -85,
    /** Streams pipe error */
    StrPipe = -86,
    /** Too many users */
    Users = -87,
    /** Socket operation on non-socket */
    NotSock = -88,
    /** Destination address required */
    DestAddrReq = -89,
    /** Message too long */
    MsgSize = -90,
    /** Protocol wrong type for socket */
    ProtoType = -91,
    /** Protocol not available */
    NoProtoOpt = -92,
    /** Protocol not supported */
    ProtoNoSupport = -93,
    /** Socket type not supported */
    SockTNoSupport = -94,
    /** Operation not supported */
    OpNotSupp = -95,
    /** Protocol family not supported */
    PFNoSupport = -96,
    /** Address family not supported by protocol */
    AFNoSupport = -97,
    /** Address already in use */
    AddrInUse = -98,
    /** Cannot assign requested address */
    AddrNotAvail = -99,
    /** Network is down */
    NetDown = -100,
    /** Network is unreachable */
    NetUnreach = -101,
    /** Network dropped connection on reset */
    NetReset = -102,
    /** Software caused connection abort */
    ConnAborted = -103,
    /** Connection reset by peer */
    ConnReset = -104,
    /** No buffer space available */
    NoBufs = -105,
    /** Transport endpoint is already connected */
    IsConn = -106,
    /** Transport endpoint is not connected */
    NotConn = -107,
    /** Cannot send after transport endpoint shutdown */
    Shutdown = -108,
    /** Too many references: cannot splice */
    TooManyRefs = -109,
    /** Connection timed out */
    TimedOut = -110,
    /** Connection refused */
    ConnRefused = -111,
    /** Host is down */
    HostDown = -112,
    /** No route to host */
    HostUnreach = -113,
    /** Operation already in progress */
    Already = -114,
    /** Operation now in progress */
    InProgress = -115,
    /** Stale file handle */
    Stale = -116,
    /** Structure needs cleaning */
    UClean = -117,
    /** Not a XENIX named type file */
    NotNam = -118,
    /** No XENIX semaphores available */
    NAvail = -119,
    /** Is a named type file */
    IsNam = -120,
    /** Remote I/O error */
    RemoteIO = -121,
    /** Disk quota exceeded */
    DQuot = -122,
    /** No medium found */
    NoMedium = -123,
    /** Wrong medium type */
    MediumType = -124,
    /** Operation cancelled */
    Canceled = -125,
    /** Required key not available */
    NoKey = -126,
    /** Key has expired */
    KeyExpired = -127,
    /** Key has been revoked */
    KeyRevoked = -128,
    /** Key was rejected by service */
    KeyRejected = -129,
    /** Owner died */
    OwnerDead = -130,
    /** State not recoverable */
    NotRecoverable = -131,
    /** Operation not possible due to RF-kill */
    RFKill = -132,
    /** Memory page has hardware error */
    HWPoison = -133,
}

impl KernelError {
    pub fn from_code(code: i32) -> Self {
        if (-133..=-1).contains(&code) && code != -41 && code != -58 {
            unsafe { core::mem::transmute::<i32, KernelError>(code) }
        } else {
            KernelError::IO
        }
    }
}

pub type KResult<T> = core::result::Result<T, KernelError>;

pub trait KResultExt<T> {
    fn consume_err(self, msg: &'static str) -> Option<T>;
}

impl<T> KResultExt<T> for KResult<T> {
    fn consume_err(self, msg: &'static str) -> Option<T> {
        if let Err(ref e) = self {
            pr_warn!("{}: {:?}", msg, e);
            None
        } else {
            self.ok()
        }
    }
}
