CREATE TABLE channels(
    id BIGSERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    frequency_hz BIGINT NOT NULL,
    modulation modulation NOT NULL,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),

    UNIQUE(frequency_hz, modulation)
);
