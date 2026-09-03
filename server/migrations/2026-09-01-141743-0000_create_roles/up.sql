CREATE TABLE roles(
    id BIGSERIAL PRIMARY KEY,
    name VARCHAR(32) NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT ''
);

INSERT INTO roles(id, name, description) VALUES
    (1, 'administrator', 'Fuld systemoversigt, administrerer brugere og noder'),
    (2, 'operator', 'Overvåger radioflåden og styrer alarmer'),
    (3, 'technician', 'Modtager arbejdsordrer og verificerer alarmer i marken'),
    (4, 'reader', 'Læseadgang til målinger og alarmer');

SELECT setval('roles_id_seq', 4);
