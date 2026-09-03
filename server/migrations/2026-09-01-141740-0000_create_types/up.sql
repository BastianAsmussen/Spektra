CREATE TYPE modulation AS ENUM ('fm', 'dab');

CREATE TYPE metric AS ENUM (
    'signal_strength',
    'signal_to_noise',
    'carrier_offset',
    'demod_error_rate',
    'spectrum_occupancy'
);

CREATE TYPE alarm_state AS ENUM ('open', 'acknowledged', 'under_verification', 'closed');

CREATE TYPE work_order_status AS ENUM ('assigned', 'completed');

CREATE TYPE rollup_resolution AS ENUM ('hourly', 'daily', 'weekly');
