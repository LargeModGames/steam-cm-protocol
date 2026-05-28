pub const PROTO_MASK: u32 = 0x8000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EMsg {
    Multi = 1,
    ServiceMethodResponse = 147,
    ClientHeartBeat = 703,
    ClientLogon = 5514,
    ClientLogOnResponse = 751,
    ServiceMethodCallFromClient = 9802,
    ServiceMethodCallFromClientNonAuthed = 9804,
    ClientHello = 9805,
}

impl EMsg {
    pub fn raw(self) -> u32 {
        self as u32
    }

    pub fn protobuf(self) -> u32 {
        self.raw() | PROTO_MASK
    }
}
