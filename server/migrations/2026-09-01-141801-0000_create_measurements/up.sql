CREATE TABLE measurements(
    id BIGSERIAL,
    node_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    metric metric NOT NULL,
    window_start TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
    window_end TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
    min DOUBLE PRECISION NOT NULL,
    max DOUBLE PRECISION NOT NULL,
    mean DOUBLE PRECISION NOT NULL,
    median DOUBLE PRECISION NOT NULL,
    stddev DOUBLE PRECISION NOT NULL,
    p95 DOUBLE PRECISION NOT NULL,
    sample_count BIGINT NOT NULL,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    PRIMARY KEY(id, window_start),
    CONSTRAINT fk_measurements_node FOREIGN KEY(node_id) REFERENCES nodes(id) ON DELETE CASCADE,
    CONSTRAINT fk_measurements_channel FOREIGN KEY(channel_id) REFERENCES channels(id),
    CONSTRAINT ck_measurements_window CHECK (window_end > window_start),
    CONSTRAINT ck_measurements_samples CHECK (sample_count > 0)
) PARTITION BY RANGE (window_start);

CREATE TABLE measurements_default PARTITION OF measurements DEFAULT;

CREATE UNIQUE INDEX idx_measurements_lookup ON measurements(node_id, channel_id, metric, window_start);
