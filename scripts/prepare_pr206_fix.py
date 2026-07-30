from pathlib import Path


path = Path("scripts/apply_pr206_receipt_fix.py")
text = path.read_text()
text = text.replace(
    "use std::collections::{BTreeMap, BTreeSet};",
    "use std::collections::BTreeMap;",
)
text = text.replace(
    "PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize",
    "PartialEq, Eq, Serialize, Deserialize",
)
text = text.replace(
    "BTreeSet<CompletedIncomingTransit>",
    "Vec<CompletedIncomingTransit>",
)
text = text.replace("BTreeSet::new()", "Vec::new()")
text = text.replace(
    "std::collections::BTreeSet<CompletedIncomingTransit>",
    "Vec<CompletedIncomingTransit>",
)
path.write_text(text)
