//! Scoped packet capture for a tentative server action. Each scope owns its packets until
//! the game journal commits it; unwinding discards them without publishing a prefix.
use super::PlayerSender;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

thread_local! {
    static SCOPES: RefCell<Vec<Packets>> = const { RefCell::new(Vec::new()) };
}

/// Encoded packets in send order, bound to their original connections.
#[derive(Default)]
pub struct Packets(Vec<(PlayerSender, Vec<u8>)>);
impl Packets {
    /// Publish only after the outermost transaction has committed.
    pub fn publish(self) {
        for (sender, body) in self.0 {
            sender.send(body);
        }
    }
}

/// A game-thread scope. Dropping it without `finish` discards tentative packets.
pub struct Scope {
    open: bool,
    _thread: PhantomData<Rc<()>>,
}
impl Scope {
    /// Begin capture before any tentative operation can send a packet.
    pub fn begin() -> Self {
        SCOPES.with(|s| s.borrow_mut().push(Packets::default()));
        Self {
            open: true,
            _thread: PhantomData,
        }
    }
    /// Transactions nested inside a capturing action also isolate their packets.
    pub fn nested() -> Option<Self> {
        active().then(Self::begin)
    }
    /// Merge a successful child in send order, or transfer the outer scope to the journal.
    pub fn finish(mut self) -> Option<Packets> {
        self.open = false;
        SCOPES.with(|s| {
            let mut scopes = s.borrow_mut();
            let packets = scopes.pop().expect("packet scope is current");
            match scopes.last_mut() {
                Some(parent) => {
                    parent.0.extend(packets.0);
                    None
                }
                None => Some(packets),
            }
        })
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.open {
            SCOPES.with(|s| {
                s.borrow_mut().pop().expect("packet scope is current");
            });
        }
    }
}

pub(super) fn active() -> bool {
    SCOPES.with(|s| !s.borrow().is_empty())
}
pub(super) fn capture(sender: &PlayerSender, body: Vec<u8>) -> Result<(), Vec<u8>> {
    SCOPES.with(|s| {
        let mut scopes = s.borrow_mut();
        match scopes.last_mut() {
            Some(packets) => {
                packets.0.push((sender.clone(), body));
                Ok(())
            }
            None => Err(body),
        }
    })
}
