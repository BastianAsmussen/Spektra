CREATE TABLE rollups(
    id BIGSERIAL PRIMARY KEY,
    node_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    metric metric NOT NULL,
    resolution rollup_resolution NOT NULL,
    bucket_start TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
    bucket_end TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
    min DOUBLE PRECISION NOT NULL,
    max DOUBLE PRECISION NOT NULL,
    mean DOUBLE PRECISION NOT NULL,
    median DOUBLE PRECISION NOT NULL,
    stddev DOUBLE PRECISION NOT NULL,
    sample_count BIGINT NOT NULL,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    UNIQUE(node_id, channel_id, metric, resolution, bucket_start),
    CONSTRAINT fk_rollups_node FOREIGN KEY(node_id) REFERENCES nodes(id) ON DELETE CASCADE,
    CONSTRAINT fk_rollups_channel FOREIGN KEY(channel_id) REFERENCES channels(id),
    CONSTRAINT ck_rollups_window CHECK (bucket_end > bucket_start)
);

CREATE INDEX idx_rollups_lookup ON rollups(node_id, channel_id, metric, resolution, bucket_start);

CREATE INDEX idx_rollups_bucket ON rollups(resolution, bucket_start);
