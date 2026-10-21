//! Side-by-side placement of overlapping timed events in a day column.

/// Placement of one item: column `col` of `cols` equal columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub col: usize,
    pub cols: usize,
}

/// `spans` are `[start, end)` minutes within the day. Items that overlap (directly or through a
/// chain of overlaps) share a cluster and split its width; each takes the leftmost free column.
/// Returns one slot per input, in input order.
pub fn place(spans: &[(i32, i32)]) -> Vec<Slot> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, -(spans[i].1)));
    let mut slots = vec![Slot { col: 0, cols: 1 }; spans.len()];
    let mut cluster: Vec<usize> = vec![];
    let mut column_ends: Vec<i32> = vec![];
    let mut cluster_end = i32::MIN;

    let close = |cluster: &mut Vec<usize>, column_ends: &mut Vec<i32>, slots: &mut Vec<Slot>| {
        for &i in cluster.iter() {
            slots[i].cols = column_ends.len().max(1);
        }
        cluster.clear();
        column_ends.clear();
    };

    for i in order {
        let (start, end) = spans[i];
        let end = end.max(start + 1);
        if !cluster.is_empty() && start >= cluster_end {
            close(&mut cluster, &mut column_ends, &mut slots);
            cluster_end = i32::MIN;
        }
        let col = match column_ends.iter().position(|&e| e <= start) {
            Some(c) => {
                column_ends[c] = end;
                c
            }
            None => {
                column_ends.push(end);
                column_ends.len() - 1
            }
        };
        slots[i].col = col;
        cluster.push(i);
        cluster_end = cluster_end.max(end);
    }
    close(&mut cluster, &mut column_ends, &mut slots);
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(col: usize, cols: usize) -> Slot {
        Slot { col, cols }
    }

    #[test]
    fn disjoint_events_take_full_width() {
        assert_eq!(place(&[(0, 60), (60, 120)]), vec![s(0, 1), s(0, 1)]);
    }

    #[test]
    fn chain_of_overlaps_shares_one_cluster() {
        // 10:00–11:30, 10:30–11:00, 11:00–12:00 (the demo Thursday).
        assert_eq!(
            place(&[(600, 690), (630, 660), (660, 720)]),
            vec![s(0, 2), s(1, 2), s(1, 2)]
        );
    }

    #[test]
    fn three_way_overlap_and_input_order_preserved() {
        assert_eq!(
            place(&[(30, 90), (0, 120), (60, 70)]),
            vec![s(1, 3), s(0, 3), s(2, 3)]
        );
    }

    #[test]
    fn zero_length_events_get_a_slot() {
        assert_eq!(place(&[(60, 60), (60, 60)]), vec![s(0, 2), s(1, 2)]);
    }
}
