//! The player's own record in a world-state frame (`0x8002`): who is playing, and in which
//! guild. Root `{ 1: { 1: uid, 3: { 1: uid, 2: uid }, 13: { 1: guild id, .. }, .. } }`.
//!
//! Read from a raw capture (2026-11-08, one player, about an hour of play): the record came
//! in 161 of 260 world-state frames, always with the same uid, and its first field 3 names
//! the same uid twice. A record without field 13 is a player in no guild.

/// Who is playing, and in which guild (`None`: in none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnPlayer {
    pub uid: u64,
    pub guild_id: Option<u64>,
}

use crate::protocol::decoder::{field_end, read_varint, Fields, Value};

/// The own-player record in a world-state root, if it has one.
pub fn own_player(root: &[u8]) -> Option<OwnPlayer> {
    if root.first() != Some(&0x0A) {
        return None;
    }
    let (len, read) = read_varint(root.get(1..)?);
    if read == 0 {
        return None;
    }
    let start = 1 + read;
    let body = &root[start..field_end(start, len, root.len())];
    let record = bytes_of(body, 1)?;
    let uid = int_of(record, 1).filter(|&uid| uid != 0)?;
    let names = bytes_of(record, 3)?;
    if int_of(names, 1) != Some(uid) || int_of(names, 2) != Some(uid) {
        return None;
    }
    let guild_id = bytes_of(record, 13)
        .and_then(|guild| int_of(guild, 1))
        .filter(|&id| id != 0);
    Some(OwnPlayer { uid, guild_id })
}

fn bytes_of(message: &[u8], number: u64) -> Option<&[u8]> {
    Fields::new(message).find_map(|f| match f.value {
        Value::Bytes(bytes) if f.number() == number => Some(bytes),
        _ => None,
    })
}

fn int_of(message: &[u8], number: u64) -> Option<u64> {
    Fields::new(message).find_map(|f| match f.value {
        Value::Varint(v) if f.number() == number => Some(v),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn int(field: u8, v: u64) -> Vec<u8> {
        [vec![field << 3], varint(v)].concat()
    }

    fn msg(field: u8, body: &[u8]) -> Vec<u8> {
        [
            vec![(field << 3) | 2],
            varint(body.len() as u64),
            body.to_vec(),
        ]
        .concat()
    }

    /// `{ 1: record, .. }` as a root (`0x0A` first), with other fields beside the record.
    fn root(record: &[u8]) -> Vec<u8> {
        msg(1, &[msg(1, record), int(2, 7), msg(6, &int(1, 5))].concat())
    }

    fn record(uid: u64, guild: Option<u64>) -> Vec<u8> {
        let mut r = [
            int(1, uid),
            int(2, 3),
            msg(3, &[int(1, uid), int(2, uid)].concat()),
        ]
        .concat();
        if let Some(id) = guild {
            r.extend(msg(13, &[int(1, id), int(3, 16)].concat()));
        }
        r.extend(msg(12, &msg(8, &int(1, uid))));
        r
    }

    #[test]
    fn the_record_gives_the_uid_and_the_guild() {
        assert_eq!(
            own_player(&root(&record(424_242, Some(77_001)))),
            Some(OwnPlayer {
                uid: 424_242,
                guild_id: Some(77_001)
            })
        );
    }

    #[test]
    fn a_record_without_field_13_is_a_player_in_no_guild() {
        assert_eq!(
            own_player(&root(&record(424_242, None))),
            Some(OwnPlayer {
                uid: 424_242,
                guild_id: None
            })
        );
    }

    #[test]
    fn a_record_whose_field_3_names_someone_else_is_not_the_players_own() {
        let other = [
            int(1, 424_242),
            msg(3, &[int(1, 9), int(2, 9)].concat()),
            msg(13, &int(1, 77_001)),
        ]
        .concat();
        assert_eq!(own_player(&root(&other)), None);
    }

    #[test]
    fn a_root_without_a_record_has_no_player() {
        let no_record = msg(1, &[int(2, 7), msg(6, &int(1, 424_242))].concat());
        assert_eq!(own_player(&no_record), None);
        assert_eq!(own_player(&[]), None);
        assert_eq!(own_player(&[0x0A]), None);
        assert_eq!(own_player(&[0x0A, 0x05, 0x08]), None);
    }

    #[test]
    fn a_guild_id_of_zero_is_no_guild() {
        let r = [
            int(1, 424_242),
            msg(3, &[int(1, 424_242), int(2, 424_242)].concat()),
            msg(13, &int(1, 0)),
        ]
        .concat();
        assert_eq!(own_player(&root(&r)).and_then(|p| p.guild_id), None);
    }
}
