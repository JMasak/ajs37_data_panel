#[derive(PartialEq)]
pub enum ReceiveState {
    WaitingForSync,
    ReceivingAddress,
    ReceivingLength(u16),
    ReceivingData((u16, u16)),
}
