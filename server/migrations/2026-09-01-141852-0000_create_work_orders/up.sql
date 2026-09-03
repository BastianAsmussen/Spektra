CREATE TABLE work_orders(
    id BIGSERIAL PRIMARY KEY,
    alarm_id BIGINT NOT NULL,
    technician_user_id BIGINT NOT NULL,
    station_name TEXT NOT NULL,
    status work_order_status NOT NULL DEFAULT 'assigned',
    fault_present BOOLEAN,
    cause TEXT,
    action_taken TEXT,
    completed_at TIMESTAMP(0) WITHOUT TIME ZONE,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT fk_orders_alarm FOREIGN KEY(alarm_id) REFERENCES alarms(id) ON DELETE CASCADE,
    CONSTRAINT fk_orders_technician FOREIGN KEY(technician_user_id) REFERENCES users(id)
);

CREATE INDEX idx_orders_alarm ON work_orders(alarm_id);
CREATE INDEX idx_orders_technician ON work_orders(technician_user_id);
