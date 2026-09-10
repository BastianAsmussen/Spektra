CREATE TABLE nodes(
    id BIGSERIAL PRIMARY KEY,
    external_identity VARCHAR(64) UNIQUE,
    name VARCHAR(100) NOT NULL,
    latitude DOUBLE PRECISION,
    longitude DOUBLE PRECISION,
    hardware JSONB NOT NULL,
    capabilities JSONB NOT NULL,
    suspended BOOLEAN NOT NULL DEFAULT FALSE,
    report_interval_seconds INT NOT NULL DEFAULT 60,
    channel_plan_version BIGINT NOT NULL DEFAULT 0,
    owner_id BIGINT,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMP(0) WITHOUT TIME ZONE,

    CONSTRAINT fk_nodes_owner FOREIGN KEY(owner_id) REFERENCES users(id) ON DELETE SET NULL,
    CONSTRAINT ck_nodes_report_interval CHECK (report_interval_seconds BETWEEN 5 AND 300)
);

CREATE INDEX idx_nodes_identity ON nodes(external_identity);
