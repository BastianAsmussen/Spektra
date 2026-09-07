const INFO_BITS: u32 = 16;
const CHECK_BITS: u32 = 10;
/// Bits in one block, information plus checkword.
pub const BLOCK_BITS: usize = 26;
/// Blocks in one group.
pub const GROUP_BLOCKS: usize = 4;
const CHECK_MASK: u32 = 0x3FF;
/// RDS generator polynomial: `x^10 + x^8 + x^7 + x^5 + x^4 + x^3 + 1`.
const GENERATOR: u32 = 0x5B9;
const SYNC_GROUPS: usize = 2;

/// Offset word marking a block's position within its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offset {
    /// First block of every group.
    A,
    /// Second block of every group.
    B,
    /// Third block of a group of version A.
    C,
    /// Third block of a group of version B.
    CPrime,
    /// Fourth block of every group.
    D,
}

impl Offset {
    /// The ten-bit word this offset adds to a checkword.
    #[must_use]
    pub const fn word(self) -> u32 {
        match self {
            Self::A => 0x0FC,
            Self::B => 0x198,
            Self::C => 0x168,
            Self::CPrime => 0x350,
            Self::D => 0x1B4,
        }
    }

    /// Every offset word, in the order a scan should try them.
    #[must_use]
    pub const fn all() -> [Self; 5] {
        [Self::A, Self::B, Self::C, Self::CPrime, Self::D]
    }

    /// The offsets valid at a given position within a group.
    #[must_use]
    pub const fn at_position(position: usize) -> &'static [Self] {
        match position % GROUP_BLOCKS {
            0 => &[Self::A],
            1 => &[Self::B],
            2 => &[Self::C, Self::CPrime],
            _ => &[Self::D],
        }
    }
}

/// The ten-bit checkword for sixteen bits of information.
#[must_use]
pub fn checkword(info: u16) -> u32 {
    let mut register = u32::from(info).wrapping_shl(CHECK_BITS);
    let mut bit = INFO_BITS.wrapping_add(CHECK_BITS);

    while bit > CHECK_BITS {
        bit = bit.wrapping_sub(1);

        if register & 1_u32.wrapping_shl(bit) != 0 {
            register ^= GENERATOR.wrapping_shl(bit.wrapping_sub(CHECK_BITS));
        }
    }

    register & CHECK_MASK
}

/// Assemble a transmittable block from its information word and offset.
#[must_use]
pub fn encode(info: u16, offset: Offset) -> u32 {
    let check = checkword(info) ^ offset.word();

    u32::from(info).wrapping_shl(CHECK_BITS) | check
}

/// The information word of a received block.
#[must_use]
pub fn information(block: u32) -> u16 {
    u16::try_from(block.wrapping_shr(CHECK_BITS) & 0xFFFF).unwrap_or(0)
}

/// Whether a received block carries a valid checkword for `offset`.
#[must_use]
pub fn validates(block: u32, offset: Offset) -> bool {
    let expected = checkword(information(block)) ^ offset.word();

    block & CHECK_MASK == expected
}

/// Which offset a received block validates against, if any.
#[must_use]
pub fn identify(block: u32) -> Option<Offset> {
    Offset::all()
        .into_iter()
        .find(|offset| validates(block, *offset))
}

/// Read [`BLOCK_BITS`] bits starting at `position`, most significant first.
#[must_use]
pub fn read_block(bits: &[bool], position: usize) -> Option<u32> {
    let window = bits.get(position..position.saturating_add(BLOCK_BITS))?;

    Some(
        window
            .iter()
            .fold(0_u32, |block, bit| block.wrapping_shl(1) | u32::from(*bit)),
    )
}

/// Undo the differential encoding RDS applies before biphase modulation.
#[must_use]
pub fn differential_decode(transmitted: &[bool]) -> Vec<bool> {
    let mut previous = false;

    transmitted
        .iter()
        .map(|&bit| {
            let decoded = bit != previous;
            previous = bit;

            decoded
        })
        .collect()
}

/// Where a bitstream's group structure starts, if it has one.
#[must_use]
pub fn find_sync(bits: &[bool]) -> Option<usize> {
    let stride = BLOCK_BITS.saturating_mul(GROUP_BLOCKS);

    (0..stride).find(|start| {
        (0..SYNC_GROUPS.saturating_mul(GROUP_BLOCKS)).all(|index| {
            let position = start.saturating_add(index.saturating_mul(BLOCK_BITS));

            read_block(bits, position).is_some_and(|block| {
                Offset::at_position(index)
                    .iter()
                    .any(|offset| validates(block, *offset))
            })
        })
    })
}

/// One decode of a bitstream: how many blocks were read and how many failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decode {
    /// Blocks that passed the CRC.
    pub good: u64,
    /// Blocks that failed the CRC.
    pub bad: u64,
}

impl Decode {
    /// Total blocks read.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.good.saturating_add(self.bad)
    }

    /// Share of blocks that failed, from 0.0 to 1.0.
    #[must_use]
    pub fn error_rate(&self) -> Option<f64> {
        let total = self.total();
        if total == 0 {
            return None;
        }

        Some(ratio(self.bad, total))
    }
}

/// Decode a bitstream from its group alignment onwards.
#[must_use]
pub fn decode(bits: &[bool]) -> Option<Decode> {
    let start = find_sync(bits)?;
    let mut result = Decode::default();

    let mut index: usize = 0;
    while let Some(block) = read_block(bits, start.saturating_add(index.saturating_mul(BLOCK_BITS)))
    {
        let valid = Offset::at_position(index)
            .iter()
            .any(|offset| validates(block, *offset));

        if valid {
            result.good = result.good.saturating_add(1);
        } else {
            result.bad = result.bad.saturating_add(1);
        }

        index = index.saturating_add(1);
    }

    Some(result)
}

fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }

    let part = super::convert::index_to_f64(usize::try_from(part).unwrap_or(usize::MAX));
    let whole = super::convert::index_to_f64(usize::try_from(whole).unwrap_or(usize::MAX));

    part / whole
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;

            self.0
        }

        fn next_u16(&mut self) -> u16 {
            u16::try_from(self.next() & 0xFFFF).unwrap_or(0)
        }
    }

    fn push_block(bits: &mut Vec<bool>, block: u32) {
        for position in (0..BLOCK_BITS).rev() {
            let shift = u32::try_from(position).unwrap_or(0);
            bits.push(block & 1_u32.wrapping_shl(shift) != 0);
        }
    }

    fn stream(groups: usize, lead: usize) -> Vec<bool> {
        let mut random = Random(0x2545_F491_4F6C_DD1D);
        let mut bits: Vec<bool> = (0..lead).map(|_| random.next() & 1 == 1).collect();

        for _ in 0..groups {
            for position in 0..GROUP_BLOCKS {
                let offset = Offset::at_position(position)
                    .first()
                    .copied()
                    .unwrap_or(Offset::A);

                push_block(&mut bits, encode(random.next_u16(), offset));
            }
        }

        bits
    }

    #[test]
    fn a_checkword_is_ten_bits() {
        let mut random = Random(7);

        for _ in 0..1000 {
            assert!(checkword(random.next_u16()) <= CHECK_MASK);
        }
    }

    #[test]
    fn the_crc_is_linear_over_the_generator() {
        assert_eq!(checkword(0), 0);
    }

    #[test]
    fn an_encoded_block_validates_against_its_own_offset() {
        let mut random = Random(11);

        for _ in 0..1_000 {
            let info = random.next_u16();
            for offset in Offset::all() {
                assert!(validates(encode(info, offset), offset));
                assert_eq!(information(encode(info, offset)), info);
            }
        }
    }

    #[test]
    fn a_block_does_not_validate_against_a_foreign_offset() {
        let block = encode(0xBEEF, Offset::A);

        assert!(validates(block, Offset::A));
        assert!(!validates(block, Offset::B));
        assert!(!validates(block, Offset::D));
    }

    #[test]
    fn a_single_bit_error_fails_the_checkword() {
        let mut random = Random(13);

        for _ in 0..200 {
            let block = encode(random.next_u16(), Offset::B);

            for bit in 0..BLOCK_BITS {
                let shift = u32::try_from(bit).unwrap_or(0);
                let corrupted = block ^ 1_u32.wrapping_shl(shift);

                assert!(
                    !validates(corrupted, Offset::B),
                    "a flip at bit {bit} went undetected"
                );
            }
        }
    }

    #[test]
    fn differential_decoding_is_immune_to_inversion() {
        let mut random = Random(17);
        let data: Vec<bool> = (0..256).map(|_| random.next() & 1 == 1).collect();

        let mut previous = false;
        let transmitted: Vec<bool> = data
            .iter()
            .map(|&bit| {
                previous = bit != previous;

                previous
            })
            .collect();
        let inverted: Vec<bool> = transmitted.iter().map(|bit| !bit).collect();

        assert_eq!(differential_decode(&transmitted), data);
        assert_eq!(
            differential_decode(&inverted).get(1..),
            data.get(1..),
            "an inverted subcarrier must decode to the same data after the first bit"
        );
    }

    #[test]
    fn a_clean_stream_synchronises_wherever_it_starts() {
        let leads: [usize; 6] = [0, 1, 7, 25, 26, 63];
        for lead in leads {
            let bits = stream(8, lead);

            assert_eq!(
                find_sync(&bits),
                Some(lead),
                "a stream led by {lead} junk bits did not synchronise there"
            );
        }
    }

    #[test]
    fn a_clean_stream_decodes_without_errors() {
        let decode = decode(&stream(16, 5)).expect("the stream synchronises");

        assert_eq!(decode.bad, 0);
        assert_eq!(decode.good, 64);
        assert_eq!(decode.error_rate(), Some(0.0));
    }

    #[test]
    fn corrupted_blocks_show_up_in_the_error_rate() {
        let mut bits = stream(16, 0);

        let corrupted = 16 - SYNC_GROUPS;
        for group in SYNC_GROUPS..16 {
            let position = group * GROUP_BLOCKS * BLOCK_BITS + 3 * BLOCK_BITS + 4;
            if let Some(bit) = bits.get_mut(position) {
                *bit = !*bit;
            }
        }

        let decode = decode(&bits).expect("the stream synchronises on the clean groups");

        assert_eq!(decode.total(), 64);
        assert_eq!(decode.bad, u64::try_from(corrupted).unwrap_or(0));
        let expected = super::super::convert::index_to_f64(corrupted) / 64.0;
        assert!((decode.error_rate().unwrap_or_default() - expected).abs() < 1e-9);
    }

    #[test]
    fn noise_never_synchronises() {
        let mut random = Random(0x1234_5678_9ABC_DEF0);
        let noise: Vec<bool> = (0..4096).map(|_| random.next() & 1 == 1).collect();

        assert_eq!(find_sync(&noise), None);
        assert_eq!(decode(&noise), None);
    }

    #[test]
    fn an_empty_decode_has_no_error_rate() {
        assert_eq!(Decode::default().error_rate(), None);
        assert_eq!(decode(&[]), None);
    }
}
