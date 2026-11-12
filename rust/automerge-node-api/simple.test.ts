import assert from "node:assert/strict";
import { describe, it } from "node:test";

import * as lib from "@automerge/automerge-node-api";

describe("Automerge", () => {
  it("can load", () => {
    const Automerge = lib.create();
  });

  it("throws on invalid actor", () => {
    // Thrown as a RangeError, like the WebAssembly binding does
    assert.throws(
      () => {
        lib.create({ actor: "invalid-actor" });
      },
      (err: unknown) =>
        err instanceof RangeError &&
        err.message ===
          "could not parse Actor ID as a hex string: Odd number of digits",
    );
  });

  it("can put", () => {
    const doc = lib.create();
    doc.put("/", "prop1", 100);
    assert.strictEqual(doc.get("/", "prop1"), 100);
  });
});
