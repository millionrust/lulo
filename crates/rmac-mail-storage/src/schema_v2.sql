CREATE TABLE outbox (
    id INTEGER PRIMARY KEY,
    envelope_from TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','sending','held'))
);
CREATE TABLE outbox_recipients (
    outbox_id INTEGER NOT NULL REFERENCES outbox(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    address TEXT NOT NULL,
    PRIMARY KEY(outbox_id,position)
);
