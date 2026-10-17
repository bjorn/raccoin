# Import a Bitcoin wallet by output descriptor

Choose **Add Source → Bitcoin Output Descriptor(s)** in a wallet. Paste the
wallet's public output descriptor and give the source a name. Separate multiple
descriptors with spaces or newlines; do not put whitespace inside a descriptor.
Use the wallet's watch-only/public export, never its recovery phrase or private
descriptor. Private keys are rejected.

Descriptors specify the script type and derivation paths explicitly, so an
`xpub` does not need to be converted to `ypub` or `zpub`. Parsing and child-key
derivation use `rust-miniscript`, including checksum validation when an exported
descriptor contains `#checksum`.

Supported address-producing descriptors include `pkh`, `wpkh`, `sh(wpkh)`,
key-path/script-path `tr`, `wsh(multi)` and `wsh(sortedmulti)`, including the
`sh(wsh(...))` wrappers used by nested SegWit multisig wallets. Bare scripts that
have no standard address are rejected. This is a watch-only import for tax
reporting; Raccoin does not sign or spend from the descriptor.

For a multisig wallet, export the complete wallet descriptor with **every**
cosigner public key. Importing just one cosigner's xpub scans different addresses.
These schematic examples show the structure; replace the placeholders with your
wallet's public export rather than copying them as addresses:

```text
wsh(sortedmulti(2,<cosigner-A-xpub>/0/*,<cosigner-B-xpub>/0/*))
sh(wsh(sortedmulti(2,<cosigner-A-xpub>/0/*,<cosigner-B-xpub>/0/*)))
```

The `/*` wildcard scans child indexes. Each wildcard key in a multisig descriptor
uses the same child index, and `sortedmulti` sorts the **derived** public keys.
Import both the receive and change descriptors, or a multipath export such as
`wpkh(<account-xpub>/<0;1>/*)`; both branches of a multipath descriptor are scanned.
A descriptor without a wildcard scans one address. Hardened derivation after an
xpub, testnet extended keys, and mixed-network extended keys are rejected.

Each ranged branch starts at index zero and stops after **20 consecutive unused
addresses**. Either confirmed or mempool activity resets that gap, while only
confirmed transactions are imported. An API error aborts the sync instead of
being treated as an unused address. A wallet with a larger unused gap may have
later transactions that are not discovered. This importer currently uses Bitcoin mainnet and the public
Blockstream Esplora API, as do the other Bitcoin source types. The service sees
the queried script hashes; use this only if that privacy tradeoff is acceptable.

References: [BIP 380](https://github.com/bitcoin/bips/blob/master/bip-0380.mediawiki),
[BIP 382](https://github.com/bitcoin/bips/blob/master/bip-0382.mediawiki), and
[BIP 383](https://github.com/bitcoin/bips/blob/master/bip-0383.mediawiki).
