CREATE TABLE node_credentials(
    id BIGSERIAL PRIMARY KEY,
    node_id BIGINT NOT NULL,
    token VARCHAR(255) NOT NULL UNIQUE,
    created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMP(0) WITHOUT TIME ZONE,
    revoked_at TIMESTAMP(0) WITHOUT TIME ZONE,

    CONSTRAINT fk_credentials_node FOREIGN KEY(node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE INDEX idx_credentials_token ON node_credentials(token);

CREATE UNIQUE INDEX idx_credentials_live ON node_credentials(node_id)
    WHERE revoked_at IS NULL;
