CREATE TABLE places (
  internal_id                  SERIAL PRIMARY KEY,
  location_latitude            FLOAT,
  location_longitude           FLOAT,
  viewport_northeast_latitude  FLOAT,
  viewport_northeast_longitude FLOAT,
  viewport_southwest_latitude  FLOAT,
  viewport_southwest_longitude FLOAT,
  business_status              TEXT,
  name                         TEXT,
  place_id                     TEXT UNIQUE,
  reference                    TEXT,
  types                        TEXT[],
  vicinity                     TEXT,
  opening_hours                JSON
);

