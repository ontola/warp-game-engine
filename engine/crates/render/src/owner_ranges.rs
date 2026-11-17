//! Immutable ownership queries used by streamed instance updates.
//! A sorted index avoids scanning the whole forest for every terrain patch.
use std::sync::OnceLock;
pub(crate) fn contains(sorted: &[usize], first: usize, count: usize) -> bool {
    static INDEXED: OnceLock<bool> = OnceLock::new();
    if *INDEXED.get_or_init(||crate::switches::OWNER_RANGE_INDEX.on()) {
        indexed(sorted,first,count)
    } else {
        let end=first.saturating_add(count);
        sorted.iter().any(|&owner|owner>=first && owner<end)
    }
}
fn indexed(sorted: &[usize], first: usize, count: usize) -> bool {
    if count==0 {return false;}
    let at=sorted.partition_point(|&owner|owner<first);
    sorted.get(at).is_some_and(|&owner|owner<first.saturating_add(count))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_matches_scan_for_holes_boundaries_empty_and_overflow() {
        for owners in [vec![],vec![0],vec![1,4,4,9,100,usize::MAX-2]] {
            for first in (0..110).chain([usize::MAX-3,usize::MAX-1,usize::MAX]) {
                for count in [0,1,2,5,200,usize::MAX] {
                    let end=first.saturating_add(count);
                    assert_eq!(indexed(&owners,first,count),owners.iter().any(|&x|x>=first&&x<end),"{owners:?} {first} {count}");
                }
            }
        }
    }
}
