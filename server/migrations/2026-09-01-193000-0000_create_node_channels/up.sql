CREATE TABLE node_channels(
    id BIGSERIAL PRIMARY KEY,
    node_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    bandwidth_hz INTEGER,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT fk_node_channels_node FOREIGN KEY(node_id) REFERENCES nodes(id) ON DELETE CASCADE,
    CONSTRAINT fk_node_channels_channel FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE,
    CONSTRAINT chk_node_channels_bandwidth CHECK (bandwidth_hz IS NULL OR bandwidth_hz > 0),

    UNIQUE(node_id, channel_id)
);

CREATE INDEX idx_node_channels_node ON node_channels(node_id);

CREATE FUNCTION bump_channel_plan_version() RETURNS TRIGGER AS $$
BEGIN
    UPDATE nodes
    SET channel_plan_version = channel_plan_version + 1
    WHERE id = COALESCE(NEW.node_id, OLD.node_id);

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_node_channels_plan_version
    AFTER INSERT OR UPDATE OR DELETE ON node_channels
    FOR EACH ROW EXECUTE FUNCTION bump_channel_plan_version();
