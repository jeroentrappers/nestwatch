# Self-hosting the Nestwatch relay server

> **Status:** `nestwatch-server` (`tools/nestwatch-server`) is available;
> relay support in the station and listener apps is being integrated. Run
> `nestwatch-server --help` for every option and its environment variable
> (`NESTWATCH_SERVER_*`); a TOML file can be passed with `--config`.

The relay is optional. Stations and listeners work on the home network and
over your own VPN without it. It helps on networks where devices cannot reach
each other directly (carrier-grade NAT, client isolation, HTTPS-only
firewalls) by

1. letting a listener reach a station through a mailbox, without knowing its
   address (rendezvous),
2. helping both sides open a direct UDP/QUIC path (hole punching, with STUN
   address discovery on UDP 3478), and
3. forwarding the encrypted byte stream when no direct path forms (relay).

The server never sees content: the Noise handshake between the two devices
runs end to end through it. What an operator can and cannot learn is listed
in [rendezvous.md §7](../specs/rendezvous.md#7-privacy-and-security); tell
your users.

## 1. What you need

- A Linux host (x86_64) reachable from the internet, with a DNS name, for
  example `relay.example.org`.
- A reverse proxy that terminates TLS and passes WebSocket upgrades:
  nginx or Caddy (both shown below).
- Firewall: **TCP 443** (HTTPS and the `/nwr/1` WebSocket) and **UDP 3478**
  (STUN binding requests, answered by `nestwatch-server` directly). TCP 80 as
  well if your proxy obtains certificates with the HTTP challenge.
- No database. The server keeps everything in memory except the mailbox
  secret hashes and transfer counters, which live in one state directory so
  a restart cannot let a mailbox be taken over.

## 2. Layout

| Path | Contents |
|------|----------|
| `/usr/local/bin/nestwatch-server` | the server binary (`nestwatch-server-linux-x86_64` from a release) |
| `/var/lib/nestwatch-server/` | state: mailbox secret hashes, transfer counters (owned by the service user, created by systemd) |
| `/var/www/nestwatch/` | static files served by the proxy: the website (`nestwatch-site.tar.gz`) with the browser listener (`nestwatch-web.tar.gz`) in `app/` |

```sh
# Binary (when releases include it)
curl -fLO https://github.com/jeroentrappers/nestwatch/releases/latest/download/nestwatch-server-linux-x86_64
curl -fLO https://github.com/jeroentrappers/nestwatch/releases/latest/download/SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
sudo install -m 0755 nestwatch-server-linux-x86_64 /usr/local/bin/nestwatch-server

# Static files: website at /, browser listener at /app/
curl -fLO https://github.com/jeroentrappers/nestwatch/releases/latest/download/nestwatch-site.tar.gz
curl -fLO https://github.com/jeroentrappers/nestwatch/releases/latest/download/nestwatch-web.tar.gz
sudo mkdir -p /var/www/nestwatch
sudo tar -C /var/www/nestwatch --strip-components=1 -xzf nestwatch-site.tar.gz
sudo mkdir -p /var/www/nestwatch/app
sudo tar -C /var/www/nestwatch/app --strip-components=1 -xzf nestwatch-web.tar.gz
```

Serving the website is optional; the relay only needs `/nwr/1`. A browser
listener loaded from the relay's HTTPS origin connects to stations through
the relay only (FR-RV-5), and talk-back works there because the origin is a
secure context.

## 3. systemd unit

`/etc/systemd/system/nestwatch-server.service`:

```ini
[Unit]
Description=Nestwatch rendezvous and relay server (NWR/1)
Documentation=https://github.com/jeroentrappers/nestwatch/blob/main/docs/self-hosting.md
After=network-online.target
Wants=network-online.target

[Service]
# HTTP for the reverse proxy on loopback, STUN on UDP 3478, state in the
# directory systemd creates below. --trust-proxy takes the client address from
# the loopback proxy's X-Real-IP / X-Forwarded-For so per-address limits work.
ExecStart=/usr/local/bin/nestwatch-server \
    --http 127.0.0.1:8740 \
    --stun 0.0.0.0:3478 \
    --trust-proxy true \
    --check-origin true \
    --state-dir ${STATE_DIRECTORY} \
    --site-dir /var/www/nestwatch/site \
    --app-dir /var/www/nestwatch/app
DynamicUser=yes
StateDirectory=nestwatch-server
StateDirectoryMode=0700
Restart=on-failure
RestartSec=2

# Hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_INET AF_INET6
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
CapabilityBoundingSet=
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

UDP 3478 is above 1024, so no extra capability is needed.

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now nestwatch-server
journalctl -u nestwatch-server -f
```

The server logs aggregate counters only (registrations, pipes, bytes,
errors), never mailbox IDs or contents, and keeps IP addresses only as long
as abuse protection needs, at most 24 hours (rendezvous.md §6). Keep the
proxy's access log in line with that: the configurations below switch it
off for `/nwr/1`.

## 4. Reverse proxy

The rendezvous endpoint is `wss://<host>/nwr/1`. Relay pipes are long-lived
(up to 24 h, idle timeout 60 s with 20 s pings), so the proxy must not time
them out early.

### 4.1 Caddy

Caddy obtains certificates and passes WebSocket upgrades automatically.
`/etc/caddy/Caddyfile`:

```caddyfile
relay.example.org {
	handle /nwr/1 {
		reverse_proxy 127.0.0.1:8740 {
			# Long-lived relay pipes: never cut them on config reloads.
			stream_close_delay 24h
		}
	}
	handle {
		root * /var/www/nestwatch
		file_server
	}
	header {
		Strict-Transport-Security "max-age=31536000"
		X-Content-Type-Options nosniff
		Referrer-Policy no-referrer
	}
}
```

Caddy's access log is off unless you add a `log` directive.

### 4.2 nginx

```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name relay.example.org;

    ssl_certificate     /etc/letsencrypt/live/relay.example.org/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/relay.example.org/privkey.pem;

    add_header Strict-Transport-Security "max-age=31536000" always;
    add_header X-Content-Type-Options nosniff always;
    add_header Referrer-Policy no-referrer always;

    root /var/www/nestwatch;
    index index.html;
    error_page 404 /404.html;

    location = /nwr/1 {
        access_log off;
        proxy_pass http://127.0.0.1:8740;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_read_timeout 120s;   # > the 20 s WebSocket pings and 60 s idle timeout
        proxy_send_timeout 120s;
    }

    location / {
        try_files $uri $uri/ =404;
    }
}

server {
    listen 80;
    listen [::]:80;
    server_name relay.example.org;
    return 301 https://$host$request_uri;
}
```

## 5. Firewall

nftables (add to your `inet filter input` chain):

```nft
tcp dport { 80, 443 } accept
udp dport 3478 accept
```

ufw:

```sh
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow 3478/udp
```

If the host is behind a router or a cloud security group, open the same ports
there. The STUN port must reach `nestwatch-server` itself, not the proxy.

## 6. Check it

```sh
# The WebSocket endpoint answers the upgrade (101 Switching Protocols)
curl -si --http1.1 -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://relay.example.org/nwr/1 | head -n 1

# UDP 3478 is open (from another machine; needs a STUN client such as stunclient)
stunclient relay.example.org 3478
```

## 7. Point your devices at it

On each station, enable **Reachable through a relay server** and enter
`https://relay.example.org` (FR-RV-1, FR-RV-6). Invitations created
afterwards carry the server and the station's mailbox; listeners paired
before need a new invitation. Listeners use the relay only when direct
endpoints fail, and keep trying to switch to a direct path.

## 8. Limits

The server enforces, and should make configurable (rendezvous.md §4.3): at
most 16 concurrent relay pipes per mailbox and 4 per client IP, 2 Mbit/s per
pipe and a monthly transfer budget per mailbox, a 60 s idle timeout and 24 h
pipe lifetime, and connection-rate limits per IP for `connect` and
`register`. Audio needs less than 0.1 Mbit/s; relayed video defaults to
320×240.
