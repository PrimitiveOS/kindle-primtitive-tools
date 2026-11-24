//! Interpret the explicit time-zone transitions in the Kindle's TZif file.
//!
//! The firmware's file can have an empty POSIX footer. Keep the last known
//! offset as the stock Kindle clock does. For files with recurring POSIX
//! rules or leap seconds, let the platform's time-zone implementation apply
//! those rules.
//!
//! TZif layout: https://www.rfc-editor.org/rfc/rfc9636.html#section-3

const HEADER_SIZE: usize = 44;

fn int_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

struct Block<'a> {
    times: &'a [u8],
    indices: &'a [u8],
    types: &'a [u8],
    tail: &'a [u8],
    leap_count: usize,
}

fn block(data: &[u8], width: usize) -> Option<Block<'_>> {
    if data.get(..4)? != b"TZif" {
        return None;
    }
    let ut = int_at(data, 20)? as usize;
    let std = int_at(data, 24)? as usize;
    let leaps = int_at(data, 28)? as usize;
    let times = int_at(data, 32)? as usize;
    let types = int_at(data, 36)? as usize;
    let names = int_at(data, 40)? as usize;
    if !(1..=256).contains(&types) || (ut != 0 && ut != types) || (std != 0 && std != types) {
        return None;
    }

    let indices_at = HEADER_SIZE.checked_add(times.checked_mul(width)?)?;
    let types_at = indices_at.checked_add(times)?;
    let names_at = types_at.checked_add(types * 6)?;
    let end = names_at
        .checked_add(names)?
        .checked_add(leaps.checked_mul(width + 4)?)?
        .checked_add(std)?
        .checked_add(ut)?;
    Some(Block {
        times: data.get(HEADER_SIZE..indices_at)?,
        indices: data.get(indices_at..types_at)?,
        types: data.get(types_at..names_at)?,
        tail: data.get(end..)?,
        leap_count: leaps,
    })
}

/// A snapshot of the firmware's explicit zone transitions.
pub struct Zone {
    times: Vec<i64>,
    kinds: Vec<u8>,
    offsets: Vec<i32>,
}

impl Zone {
    /// Decode a transition table that needs no recurring rules. For version
    /// 2 or newer use its 64-bit table, including pre-1901 transitions.
    pub fn from_tzif(data: &[u8]) -> Option<Self> {
        let version = *data.get(4)?;
        let first = block(data, 4)?;
        let (table, width) = match version {
            0 => (first, 4),
            b'2'..=b'4' => {
                if first.tail.get(4) != Some(&version) {
                    return None;
                }
                let second = block(first.tail, 8)?;
                if second.tail != b"\n\n" {
                    return None;
                }
                (second, 8)
            }
            _ => return None,
        };
        if table.leap_count != 0 {
            return None;
        }
        let times: Vec<_> = table
            .times
            .chunks_exact(width)
            .map(|value| {
                if width == 4 {
                    i64::from(i32::from_be_bytes(value.try_into().unwrap()))
                } else {
                    i64::from_be_bytes(value.try_into().unwrap())
                }
            })
            .collect();
        let offsets: Vec<_> = table
            .types
            .as_chunks::<6>()
            .0
            .iter()
            .map(|value| i32::from_be_bytes(value[..4].try_into().unwrap()))
            .collect();
        if times.windows(2).any(|pair| pair[0] >= pair[1])
            || table
                .indices
                .iter()
                .any(|&kind| usize::from(kind) >= offsets.len())
            || offsets
                .iter()
                .any(|value| !(-89_999..=93_599).contains(value))
        {
            return None;
        }
        Some(Self {
            times,
            kinds: table.indices.to_vec(),
            offsets,
        })
    }

    /// Seconds east of UTC at this Unix timestamp.
    pub fn offset_at(&self, instant: i64) -> i32 {
        let offset_index = self.times.partition_point(|&time| time <= instant);
        let kind = offset_index
            .checked_sub(1)
            .map_or(0, |index| self.kinds[index]);
        self.offsets[usize::from(kind)]
    }

    /// Translate civil time, encoded as seconds as if in UTC, to a Unix
    /// timestamp. Repeated times choose the earliest candidate past the
    /// optional cutoff. Skipped times can move forward by the gap's length.
    pub fn resolve_local(&self, civil: i64, after: Option<i64>, advance_gap: bool) -> Option<i64> {
        let allowed = |instant: i64| after.is_none_or(|limit| instant > limit);
        let mut exact = self
            .offsets
            .iter()
            .filter_map(|&offset| {
                let instant = civil.checked_sub(i64::from(offset))?;
                (self.offset_at(instant) == offset).then_some(instant)
            })
            .peekable();
        if exact.peek().is_some() || !advance_gap {
            return exact.filter(|&instant| allowed(instant)).min();
        }
        let mut before = self.offsets[0];
        for (&instant, &kind) in self.times.iter().zip(&self.kinds) {
            let after = self.offsets[usize::from(kind)];
            let start = instant.checked_add(i64::from(before));
            let end = instant.checked_add(i64::from(after));
            if start
                .zip(end)
                .is_some_and(|(start, end)| (start..end).contains(&civil))
            {
                return civil
                    .checked_sub(i64::from(before))
                    .filter(|&value| allowed(value));
            }
            before = after;
        }
        None
    }
}

/// Seconds east of UTC for the supplied transition table, or `None` when the
/// table is invalid or needs the system's recurring rules.
pub fn offset_at(data: &[u8], now: i64) -> Option<i32> {
    Some(Zone::from_tzif(data)?.offset_at(now))
}

/// The offset displayed in device settings, including fractional-hour zones.
pub fn format_offset(offset: i32) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let seconds = offset.unsigned_abs();
    let (hours, minutes, remainder) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    if remainder == 0 {
        format!("UTC{sign}{hours:02}:{minutes:02}")
    } else {
        format!("UTC{sign}{hours:02}:{minutes:02}:{remainder:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 1 TZif block: `transitions` as (time, type index), `offsets` as the types' UTC offsets.
    fn tzif(transitions: &[(i32, u8)], offsets: &[i32]) -> Vec<u8> {
        let mut d = b"TZif\0".to_vec();
        d.extend([0u8; 15]);
        let charcnt = 4u32;
        for count in [
            0u32,
            0,
            0,
            transitions.len() as u32,
            offsets.len() as u32,
            charcnt,
        ] {
            d.extend(count.to_be_bytes());
        }
        for (t, _) in transitions {
            d.extend(t.to_be_bytes());
        }
        for (_, k) in transitions {
            d.push(*k);
        }
        for off in offsets {
            d.extend(off.to_be_bytes());
            d.extend([0u8, 0u8]);
        }
        d.extend(b"ABC\0");
        d
    }

    #[test]
    fn the_last_transition_in_force_gives_the_offset() {
        let data = tzif(&[(0, 1), (1_000_000, 2)], &[0, 3600, 7 * 3600]);
        assert_eq!(offset_at(&data, -5), Some(0)); // before any transition: type 0
        assert_eq!(offset_at(&data, 500), Some(3600));
        assert_eq!(offset_at(&data, 1_794_935_000), Some(7 * 3600)); // long after the last one
    }

    #[test]
    fn a_file_without_transitions_uses_its_only_type() {
        assert_eq!(
            offset_at(&tzif(&[], &[7 * 3600]), 1_794_935_000),
            Some(7 * 3600)
        );
    }

    #[test]
    fn other_data_is_not_a_zone() {
        assert_eq!(offset_at(b"GMT+7", 0), None);
        assert_eq!(offset_at(b"TZif2", 0), None);
        assert_eq!(offset_at(&tzif(&[(0, 5)], &[0]), 10), None);
    }

    #[test]
    fn local_round_trip_uses_the_last_known_offset() {
        let zone = Zone::from_tzif(&tzif(&[(0, 1)], &[0, 7 * 3600])).unwrap();
        let instant = 1_794_935_000;
        let civil = instant + i64::from(zone.offset_at(instant));
        assert_eq!(zone.resolve_local(civil, None, false), Some(instant));
        assert_eq!(zone.resolve_local(civil, Some(instant), true), None);
    }

    #[test]
    fn local_gap_and_fold_policy() {
        let zone = Zone::from_tzif(&tzif(&[(10_000, 1), (40_000, 0)], &[0, 3600])).unwrap();
        assert_eq!(zone.resolve_local(10_000, None, false), None);
        assert_eq!(zone.resolve_local(10_005, None, true), Some(10_005));
        assert_eq!(zone.resolve_local(13_600, None, false), Some(10_000));
        assert_eq!(zone.resolve_local(40_100, None, false), Some(36_500));
        assert_eq!(zone.resolve_local(40_100, Some(36_500), true), Some(40_100));
        assert_eq!(zone.resolve_local(40_100, Some(40_100), true), None);
    }

    #[test]
    fn version_two_uses_the_sixty_four_bit_table_and_delegates_nonempty_rules() {
        let mut first = tzif(&[], &[0]);
        first[4] = b'2';
        let transition = 3_000_000_000_i64;
        let mut second = tzif(&[(0, 1)], &[0, 3600]);
        second[4] = b'2';
        second.splice(44..48, transition.to_be_bytes());
        first.extend(second);
        assert!(Zone::from_tzif(&first).is_none());
        first.extend(b"\n\n");
        assert_eq!(offset_at(&first, transition - 1), Some(0));
        assert_eq!(offset_at(&first, transition), Some(3600));
        first.pop();
        first.extend(b"STD-1\n");
        assert!(Zone::from_tzif(&first).is_none());
    }

    #[test]
    fn settings_format_positive_negative_and_fractional_offsets() {
        assert_eq!(format_offset(0), "UTC+00:00");
        assert_eq!(format_offset(7 * 3600), "UTC+07:00");
        assert_eq!(format_offset(-3 * 3600 - 30 * 60), "UTC-03:30");
        assert_eq!(format_offset(5 * 3600 + 45 * 60), "UTC+05:45");
    }
}
