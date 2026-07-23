CREATE TABLE session_ports (
  session_name TEXT NOT NULL,
  host_port INTEGER NOT NULL CHECK (host_port BETWEEN 1 AND 65535),
  guest_port INTEGER NOT NULL CHECK (guest_port BETWEEN 1 AND 65535),
  protocol TEXT NOT NULL CHECK (protocol IN ('tcp', 'udp')),
  PRIMARY KEY (session_name, host_port, protocol),
  FOREIGN KEY (session_name) REFERENCES sessions(name) ON DELETE CASCADE
);
