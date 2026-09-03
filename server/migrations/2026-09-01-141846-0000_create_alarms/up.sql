CREATE TABLE alarms(
    id BIGSERIAL PRIMARY KEY,
    node_id BIGINT NOT NULL,
    channel_id BIGINT,
    metric metric,
    state alarm_state NOT NULL DEFAULT 'open',
    explanation JSONB NOT NULL,
    raised_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMP(0) WITHOUT TIME ZONE,

    CONSTRAINT fk_alarms_node FOREIGN KEY(node_id) REFERENCES nodes(id) ON DELETE CASCADE,
    CONSTRAINT fk_alarms_channel FOREIGN KEY(channel_id) REFERENCES channels(id)
);

CREATE INDEX idx_alarms_open ON alarms(node_id, channel_id, metric) WHERE state <> 'closed';
CREATE INDEX idx_alarms_raised_at ON alarms(raised_at DESC);
CREATE INDEX idx_alarms_node ON alarms(node_id);
