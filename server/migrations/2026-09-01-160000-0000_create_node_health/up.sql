CREATE TABLE node_health(
    id BIGSERIAL PRIMARY KEY,
    node_id BIGINT NOT NULL,
    measured_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
    uptime_seconds DOUBLE PRECISION,
    load_1m DOUBLE PRECISION,
    load_5m DOUBLE PRECISION,
    load_15m DOUBLE PRECISION,
    cpu_temperature_celsius DOUBLE PRECISION,
    clock_offset_seconds DOUBLE PRECISION,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    UNIQUE(node_id, measured_at),
    CONSTRAINT fk_node_health_node FOREIGN KEY(node_id) REFERENCES nodes(id) ON DELETE CASCADE,
    CONSTRAINT ck_node_health_uptime CHECK (uptime_seconds >= 0),
    CONSTRAINT ck_node_health_load CHECK (load_1m >= 0 AND load_5m >= 0 AND load_15m >= 0)
);

CREATE INDEX idx_node_health_lookup ON node_health(node_id, measured_at DESC);
