CREATE TABLE alarm_events(
    id BIGSERIAL PRIMARY KEY,
    alarm_id BIGINT NOT NULL,
    from_state alarm_state,
    to_state alarm_state NOT NULL,
    changed_by_user_id BIGINT,
    reason TEXT NOT NULL,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT fk_events_alarm FOREIGN KEY(alarm_id) REFERENCES alarms(id) ON DELETE CASCADE,
    CONSTRAINT fk_events_user FOREIGN KEY(changed_by_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_events_alarm ON alarm_events(alarm_id, created_at);
