"""Offline checks for the placement-evidence fail-closed guard."""
import importlib.util
from pathlib import Path
import unittest


MODULE = Path(__file__).with_name("placement-evidence.py")
SPEC = importlib.util.spec_from_file_location("placement_evidence", MODULE)
assert SPEC and SPEC.loader
PLACEMENT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PLACEMENT)


class PlacementRequirementTests(unittest.TestCase):
    target = {"node": "target-node"}

    def test_same_node_accepts_matching_observer(self):
        PLACEMENT.require_placement("same-node", {"node": "target-node"}, self.target)

    def test_cross_node_accepts_distinct_observer(self):
        PLACEMENT.require_placement("cross-node", {"node": "other-node"}, self.target)

    def test_same_node_rejects_distinct_observer(self):
        with self.assertRaisesRegex(RuntimeError, "refusing to label"):
            PLACEMENT.require_placement("same-node", {"node": "other-node"}, self.target)

    def test_cross_node_rejects_matching_observer(self):
        with self.assertRaisesRegex(RuntimeError, "refusing to label"):
            PLACEMENT.require_placement("cross-node", {"node": "target-node"}, self.target)

    def test_unassigned_node_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "has not assigned a node"):
            PLACEMENT.require_placement("same-node", {"node": None}, self.target)


if __name__ == "__main__":
    unittest.main()
