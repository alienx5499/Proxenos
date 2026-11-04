-- The aggregate is stored whole as JSON in deals.body_json (StoredDeal) and re-validated by
-- the domain on load. The other deals columns and the parties table are indexes rewritten
-- from that JSON on every save; they are never read back as truth. Offers, signatures,
-- sign requests, sealed limits (AES-GCM nonce + ciphertext), own-side checks and the receipt
-- live inside the JSON, so they have no tables here.

CREATE TABLE deals (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    room_id TEXT NOT NULL,
    desk_a_room_id TEXT,
    desk_b_room_id TEXT,
    state TEXT NOT NULL,
    terminal INTEGER NOT NULL,
    round INTEGER NOT NULL,
    max_rounds INTEGER NOT NULL,
    deadline INTEGER NOT NULL,
    terms_hash TEXT,
    version INTEGER NOT NULL,
    body_json TEXT NOT NULL
);
CREATE INDEX deals_room ON deals(room_id);
CREATE INDEX deals_desk_a ON deals(desk_a_room_id);
CREATE INDEX deals_desk_b ON deals(desk_b_room_id);

-- A NULL hash means the secret was used.
CREATE TABLE parties (
    deal_id TEXT NOT NULL REFERENCES deals(id),
    role TEXT NOT NULL,
    seal_token_hash TEXT UNIQUE,
    join_code_hash TEXT UNIQUE,
    PRIMARY KEY (deal_id, role)
);

CREATE TABLE deal_events (
    deal_id TEXT NOT NULL REFERENCES deals(id),
    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (deal_id, seq)
);

CREATE TABLE seen_messages (
    message_id TEXT PRIMARY KEY,
    at INTEGER NOT NULL
);
