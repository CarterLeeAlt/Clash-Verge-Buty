#[cfg(test)]
use super::IVerge;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Draft<T: Clone> {
    inner: Arc<Mutex<(T, Option<T>)>>,
}

impl<T: Clone> Draft<T> {
    pub fn data(&self) -> MappedMutexGuard<'_, T> {
        MutexGuard::map(self.inner.lock(), |guard| &mut guard.0)
    }

    pub fn latest(&self) -> MappedMutexGuard<'_, T> {
        MutexGuard::map(self.inner.lock(), |inner| {
            if inner.1.is_none() {
                &mut inner.0
            } else {
                inner.1.as_mut().unwrap()
            }
        })
    }

    pub fn draft(&self) -> MappedMutexGuard<'_, T> {
        MutexGuard::map(self.inner.lock(), |inner| {
            if inner.1.is_none() {
                inner.1 = Some(inner.0.clone());
            }

            inner.1.as_mut().unwrap()
        })
    }

    pub fn apply(&self) -> Option<T> {
        let mut inner = self.inner.lock();
        inner
            .1
            .take()
            .map(|draft| std::mem::replace(&mut inner.0, draft))
    }

    pub fn discard(&self) -> Option<T> {
        let mut inner = self.inner.lock();
        inner.1.take()
    }
}

impl<T: Clone> From<T> for Draft<T> {
    fn from(data: T) -> Self {
        Draft {
            inner: Arc::new(Mutex::new((data, None))),
        }
    }
}

#[test]
fn test_draft() {
    let verge = IVerge {
        enable_auto_launch: Some(true),
        enable_tun_mode: Some(false),
        ..IVerge::default()
    };

    let draft = Draft::from(verge);

    assert_eq!(draft.data().enable_auto_launch, Some(true));
    assert_eq!(draft.data().enable_tun_mode, Some(false));

    assert_eq!(draft.draft().enable_auto_launch, Some(true));
    assert_eq!(draft.draft().enable_tun_mode, Some(false));

    let mut d = draft.draft();
    d.enable_auto_launch = Some(false);
    d.enable_tun_mode = Some(true);
    drop(d);

    assert_eq!(draft.data().enable_auto_launch, Some(true));
    assert_eq!(draft.data().enable_tun_mode, Some(false));

    assert_eq!(draft.draft().enable_auto_launch, Some(false));
    assert_eq!(draft.draft().enable_tun_mode, Some(true));

    assert_eq!(draft.latest().enable_auto_launch, Some(false));
    assert_eq!(draft.latest().enable_tun_mode, Some(true));

    assert!(draft.apply().is_some());
    assert!(draft.apply().is_none());

    assert_eq!(draft.data().enable_auto_launch, Some(false));
    assert_eq!(draft.data().enable_tun_mode, Some(true));

    assert_eq!(draft.draft().enable_auto_launch, Some(false));
    assert_eq!(draft.draft().enable_tun_mode, Some(true));

    let mut d = draft.draft();
    d.enable_auto_launch = Some(true);
    drop(d);

    assert_eq!(draft.data().enable_auto_launch, Some(false));

    assert_eq!(draft.draft().enable_auto_launch, Some(true));

    assert!(draft.discard().is_some());

    assert_eq!(draft.data().enable_auto_launch, Some(false));

    assert!(draft.discard().is_none());

    assert_eq!(draft.draft().enable_auto_launch, Some(false));
}
