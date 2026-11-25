//! Where the players sit on the couch, for Rust games: the Game Room lobby
//! lets everyone pick a place (0 is leftmost), also mid-game. Split-screen
//! views should run left to right in that order, so whoever sits left
//! looks at the left view. Luau games get this from the `warp` crate
//! (`players.by_couch()`, `couch_changed`); Rust games read the couch of
//! each local player here and lay out their views with [`couch_views`].
//!
//! Under a Splitscreen Club party the couch comes with the seats
//! (`splitscreen_club::Seat::couch`, updated by `players_updated`). In the
//! browser and phone hosts the host sets it every tick for its seats
//! ([`set_seat_couch`]), as it does the seat devices.

static SEAT_COUCH: std::sync::Mutex<Vec<Option<i64>>> = std::sync::Mutex::new(Vec::new());

/// The couch place of each host seat (the browser and phone hosts set it
/// every tick); None: unknown.
pub fn set_seat_couch(couch: Vec<Option<i64>>) {
    if let Ok(mut c) = SEAT_COUCH.lock() && *c != couch { *c = couch; }
}

/// The couch place of host seat `seat` ([`set_seat_couch`]); None: unknown
/// or no host that tells.
pub fn seat_couch(seat: usize) -> Option<i64> { SEAT_COUCH.lock().ok()?.get(seat).copied().flatten() }

/// The players (by index) left to right on the couch: by couch place,
/// unknown places after, ties and unknowns in index order.
pub fn couch_order(couch: &[Option<i64>]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..couch.len()).collect();
    order.sort_by_key(|&i| (couch[i].unwrap_or(i64::MAX), i));
    order
}

/// The split-screen view of each player (by index): view 0 for whoever
/// sits leftmost, and so on ([`couch_order`]). Without couch places it is
/// the identity.
pub fn couch_views(couch: &[Option<i64>]) -> Vec<usize> {
    let mut views = vec![0; couch.len()];
    for (view, i) in couch_order(couch).into_iter().enumerate() { views[i] = view; }
    views
}

/// Put per-player values (cameras, blur settings...) in view order.
pub fn in_view_order<T: Clone>(per_player: &[T], couch: &[Option<i64>]) -> Vec<T> {
    let order = couch_order(&couch[..couch.len().min(per_player.len())]);
    let mut out: Vec<T> = order.iter().map(|&i| per_player[i].clone()).collect();
    out.extend(per_player.iter().skip(out.len()).cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_follow_the_couch() {
        assert_eq!(couch_views(&[Some(1), Some(0)]), [1, 0]);
        assert_eq!(couch_views(&[None, None, None]), [0, 1, 2]);
        assert_eq!(couch_views(&[None, Some(0), Some(2)]), [2, 0, 1]);
        assert_eq!(couch_order(&[Some(2), Some(0), Some(1)]), [1, 2, 0]);
        assert_eq!(in_view_order(&["a", "b"], &[Some(1), Some(0)]), ["b", "a"]);
        assert_eq!(in_view_order(&["a", "b", "c"], &[Some(1), Some(0)]), ["b", "a", "c"]);
    }
}
