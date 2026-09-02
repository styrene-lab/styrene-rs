#!/usr/bin/env python3
"""Ensure product labels keep native and Styrene-specific behavior distinct."""

from __future__ import annotations

import pathlib
import tomllib
import unittest


class ParityClaimLabelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        with pathlib.Path("product/capabilities-v1.toml").open("rb") as handle:
            cls.registry = tomllib.load(handle)

    def test_product_capabilities_defer_to_parity_claims(self) -> None:
        capabilities = {entry["id"]: entry for entry in self.registry["capabilities"]}
        self.assertIn("lxmf.direct parity claim", capabilities["messaging.lxmf.direct"]["notes"])
        self.assertIn("Native NomadNet transport", capabilities["browse.nomadnet.micron"]["notes"])

    def test_native_transport_claims_follow_hosted_live_evidence(self) -> None:
        claims = {entry["id"]: entry for entry in self.registry["parity_claims"]}
        gates = {entry["id"]: entry for entry in self.registry["parity_gates"]}
        for claim_id in ("lxmf.direct", "lxmf.resources", "lxmf.propagation", "nomadnet.transport"):
            self.assertEqual(claims[claim_id]["level"], "verified", claim_id)
            for gate_id in claims[claim_id]["required_gates"]:
                gate = gates[gate_id]
                self.assertEqual(gate["protocol"], "native", gate_id)
                self.assertFalse(gate["ignored"], gate_id)
                if gate["kind"] == "live":
                    self.assertIn("tests/interop/handoffs/pinned-live-evidence.json", gate["evidence"], gate_id)
        self.assertEqual(claims["rns.operations"]["level"], "degraded")
        self.assertIn("channel", claims["rns.operations"]["reason"])

    def test_styrene_specific_evidence_is_labelled(self) -> None:
        gates = {entry["id"]: entry for entry in self.registry["parity_gates"]}
        self.assertEqual(gates["lxmf-styrene-propagation"]["protocol"], "styrene-specific")
        self.assertEqual(gates["nomadnet-styrene-pages"]["protocol"], "styrene-specific")


if __name__ == "__main__":
    unittest.main()
