# Navigation target wire shapes

Issue #222 replaces legacy optional navigation target fields with required tagged targets shared by the Rust wire layer and the Godot client.

The intended forms are `{"Ship": id}` or `{"Gate": id}` for Approach, Orbit, and KeepAtRange, and `{"Gate": id}` or `{"Body": id}` for Warp.
