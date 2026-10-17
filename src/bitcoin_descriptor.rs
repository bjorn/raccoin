use std::str::FromStr;

use anyhow::{bail, Context, Result};
use bitcoin::{secp256k1::Secp256k1, Address, Network, NetworkKind};
use miniscript::{
    descriptor::{Wildcard, XKeyNetwork},
    Descriptor, DescriptorPublicKey,
};

pub(crate) struct BitcoinDescriptor(Descriptor<DescriptorPublicKey>);

impl BitcoinDescriptor {
    pub(crate) fn parse_list(input: &str) -> Result<Vec<Self>> {
        let mut descriptors = Vec::new();
        for text in input.split_ascii_whitespace() {
            // FromStr accepts public keys only and validates any supplied checksum.
            let descriptor = Descriptor::<DescriptorPublicKey>::from_str(text)
                .context("Invalid public Bitcoin output descriptor")?;
            match descriptor.xkey_network() {
                XKeyNetwork::NoXKeys | XKeyNetwork::Single(NetworkKind::Main) => {}
                _ => bail!("Bitcoin descriptors must use mainnet extended public keys"),
            }
            for descriptor in descriptor.into_single_descriptors()? {
                if !descriptor.iter_pk().all(|key| match key {
                    DescriptorPublicKey::Single(_) => true,
                    DescriptorPublicKey::XPub(key) => {
                        key.wildcard != Wildcard::Hardened
                            && key
                                .derivation_path
                                .into_iter()
                                .all(|child| !child.is_hardened())
                    }
                    DescriptorPublicKey::MultiXPub(_) => false,
                }) {
                    bail!("Cannot derive hardened children from public descriptor keys");
                }
                let descriptor = Self(descriptor);
                // Reject unsupported scripts or hardened public derivation before network I/O.
                descriptor.address(0)?;
                descriptors.push(descriptor);
            }
        }
        if descriptors.is_empty() {
            bail!("Enter at least one Bitcoin output descriptor");
        }
        Ok(descriptors)
    }

    pub(crate) fn has_wildcard(&self) -> bool {
        self.0.has_wildcard()
    }

    pub(crate) fn address(&self, index: u32) -> Result<Address> {
        let secp = Secp256k1::verification_only();
        Ok(self
            .0
            .at_derivation_index(index)?
            .derived_descriptor(&secp)
            .address(Network::Bitcoin)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public test vectors from BIP 382 and BIP 383 (BSD-2-Clause).
    const BIP382_XPUB: &str = "xpub69H7F5d8KSRgmmdJg2KhpAK8SR3DjMwAdkxj3ZuxV27CprR9LgpeyGmXUbC6wb7ERfvrnKZjXoUmmDznezpbZb7ap6r1D3tgFxHmwMkQTPH";
    const BIP383_XPUB_A: &str = "xpub6ERApfZwUNrhLCkDtcHTcxd75RbzS1ed54G1LkBUHQVHQKqhMkhgbmJbZRkrgZw4koxb5JaHWkY4ALHY2grBGRjaDMzQLcgJvLJuZZvRcEL";
    const BIP383_XPUB_B: &str = "xpub68NZiKmJWnxxS6aaHmn81bvJeTESw724CRDs6HbuccFQN9Ku14VQrADWgqbhhTHBaohPX4CjNLf9fq9MYo6oDaPPLPxSb7gwQN3ih19Zm4Y";
    const BIP383_SCRIPTS: [&str; 3] = [
        "5221025d5fc65ebb8d44a5274b53bac21ff8307fec2334a32df05553459f8b1f7fe1b62102fbd47cc8034098f0e6a94c6aeee8528abf0a2153a5d8e46d325b7284c046784652ae",
        "52210264fd4d1f5dea8ded94c61e9641309349b62f27fbffe807291f664e286bfbe6472103f4ece6dfccfa37b211eb3d0af4d0c61dba9ef698622dc17eecdf764beeb005a652ae",
        "5221022ccabda84c30bad578b13c89eb3b9544ce149787e5b538175b1d1ba259cbb83321024d902e1a2fc7a8755ab5b694c575fce742c48d9ff192e63df5193e4c7afe1f9c52ae",
    ];

    fn parsed(text: &str) -> BitcoinDescriptor {
        BitcoinDescriptor::parse_list(text).unwrap().remove(0)
    }

    #[test]
    fn bip382_wpkh_derivation_matches_published_scripts() {
        let descriptor = parsed(&format!("wpkh([ffffffff/13']{BIP382_XPUB}/1/2/*)"));
        let expected = [
            "0014326b2249e3a25d5dc60935f044ee835d090ba859",
            "0014af0bd98abc2f2cae66e36896a39ffe2d32984fb7",
            "00141fa798efd1cbf95cebf912c031b8a4a6e9fb9f27",
        ];
        for (index, script) in expected.iter().enumerate() {
            assert_eq!(
                descriptor
                    .address(index as u32)
                    .unwrap()
                    .script_pubkey()
                    .to_hex_string(),
                *script
            );
        }
    }

    #[test]
    fn bip383_wsh_sortedmulti_matches_published_derived_scripts() {
        let descriptor = parsed(&format!(
            "wsh(sortedmulti(2,{BIP383_XPUB_A}/*,{BIP383_XPUB_B}/0/0/*))"
        ));
        for (index, witness_script) in BIP383_SCRIPTS.iter().enumerate() {
            let expected = bitcoin::ScriptBuf::from_hex(witness_script)
                .unwrap()
                .to_p2wsh();
            assert_eq!(
                descriptor.address(index as u32).unwrap().script_pubkey(),
                expected
            );
        }
    }

    #[test]
    fn bip383_sh_wsh_sortedmulti_wraps_the_same_witness_scripts() {
        let descriptor = parsed(&format!(
            "sh(wsh(sortedmulti(2,{BIP383_XPUB_A}/*,{BIP383_XPUB_B}/0/0/*)))"
        ));
        for (index, witness_script) in BIP383_SCRIPTS.iter().enumerate() {
            let expected = bitcoin::ScriptBuf::from_hex(witness_script)
                .unwrap()
                .to_p2wsh()
                .to_p2sh();
            assert_eq!(
                descriptor.address(index as u32).unwrap().script_pubkey(),
                expected
            );
        }
    }

    #[test]
    fn sortedmulti_sorts_derived_keys_independently_of_input_order() {
        let a = parsed(&format!(
            "wsh(sortedmulti(2,{BIP383_XPUB_A}/*,{BIP383_XPUB_B}/0/0/*))"
        ));
        let b = parsed(&format!(
            "wsh(sortedmulti(2,{BIP383_XPUB_B}/0/0/*,{BIP383_XPUB_A}/*))"
        ));
        for index in 0..3 {
            assert_eq!(a.address(index).unwrap(), b.address(index).unwrap());
        }
    }

    #[test]
    fn verifies_optional_descriptor_checksum() {
        let input = format!("wpkh({BIP382_XPUB}/1/2/*)");
        let with_checksum = Descriptor::<DescriptorPublicKey>::from_str(&input)
            .unwrap()
            .to_string();
        assert_eq!(
            parsed(&input).address(0).unwrap(),
            parsed(&with_checksum).address(0).unwrap()
        );
        assert!(BitcoinDescriptor::parse_list(&format!("{input}#aaaaaaaa")).is_err());
    }

    #[test]
    fn expands_multipath_receive_and_change_branches() {
        let branches =
            BitcoinDescriptor::parse_list(&format!("wpkh({BIP382_XPUB}/<0;1>/*)")).unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(
            branches[0].address(0).unwrap(),
            parsed(&format!("wpkh({BIP382_XPUB}/0/*)"))
                .address(0)
                .unwrap()
        );
        assert_eq!(
            branches[1].address(0).unwrap(),
            parsed(&format!("wpkh({BIP382_XPUB}/1/*)"))
                .address(0)
                .unwrap()
        );
    }

    #[test]
    fn accepts_fixed_public_key_descriptor_without_a_scan_range() {
        let descriptor =
            parsed("wpkh(03a34b99f22c790c4e36b2b3c2c35a36db06226e41c692fc82b8b56ac1c540c5bd)");
        assert!(!descriptor.has_wildcard());
        assert_eq!(
            descriptor
                .address(0)
                .unwrap()
                .script_pubkey()
                .to_hex_string(),
            "00149a1c78a507689f6f54b847ad1cef1e614ee23f1e"
        );
    }

    #[test]
    fn rejects_testnet_extended_keys_before_any_lookup() {
        let mut key = bitcoin::bip32::Xpub::from_str(BIP382_XPUB).unwrap();
        key.network = NetworkKind::Test;
        assert!(BitcoinDescriptor::parse_list(&format!("wpkh({key}/0/*)")).is_err());
    }

    #[test]
    fn rejects_hardened_derivation_from_public_keys() {
        assert!(BitcoinDescriptor::parse_list(&format!("wpkh({BIP382_XPUB}/0'/*)")).is_err());
        assert!(BitcoinDescriptor::parse_list(&format!("wpkh({BIP382_XPUB}/0/*h)")).is_err());
    }

    #[test]
    fn rejects_private_key_input() {
        // This WIF is a published BIP 383 test vector, never a user wallet key.
        assert!(BitcoinDescriptor::parse_list(
            "wpkh(L4rK1yDtCWekvXuE6oXD9jCYfFNV2cWRpVuPLBcCU2z8TrisoyY1)"
        )
        .is_err());
    }

    #[test]
    fn rejects_empty_or_invalid_list_without_partial_results() {
        assert!(BitcoinDescriptor::parse_list(" \n").is_err());
        assert!(
            BitcoinDescriptor::parse_list(&format!("wpkh({BIP382_XPUB}/0/*) invalid")).is_err()
        );
    }

    #[test]
    fn accepts_multiple_descriptors_separated_by_whitespace() {
        let input = format!("wpkh({BIP382_XPUB}/0/*)\nwpkh({BIP382_XPUB}/1/*)");
        assert_eq!(BitcoinDescriptor::parse_list(&input).unwrap().len(), 2);
    }
}
