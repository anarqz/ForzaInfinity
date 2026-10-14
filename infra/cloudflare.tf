data "cloudflare_zone" "zone" {
  filter = {
    name = var.domain
  }
}

locals {
  account_id = data.cloudflare_zone.zone.account.id
}

resource "random_bytes" "tunnel_secret" {
  length = 32
}

resource "cloudflare_zero_trust_tunnel_cloudflared" "infinity" {
  account_id    = local.account_id
  name          = "forza-infinity"
  config_src    = "cloudflare"
  tunnel_secret = random_bytes.tunnel_secret.base64
}

data "cloudflare_zero_trust_tunnel_cloudflared_token" "infinity" {
  account_id = local.account_id
  tunnel_id  = cloudflare_zero_trust_tunnel_cloudflared.infinity.id
}

# The VM's cloudflared hands every request for the hostname to the server on loopback.
resource "cloudflare_zero_trust_tunnel_cloudflared_config" "infinity" {
  account_id = local.account_id
  tunnel_id  = cloudflare_zero_trust_tunnel_cloudflared.infinity.id
  config = {
    ingress = [
      {
        hostname = var.hostname
        service  = "http://127.0.0.1:8081"
      },
      {
        service = "http_status:404"
      },
    ]
  }
}

resource "cloudflare_dns_record" "infinity" {
  zone_id = data.cloudflare_zone.zone.id
  name    = var.hostname
  type    = "CNAME"
  content = "${cloudflare_zero_trust_tunnel_cloudflared.infinity.id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
  comment = "ForzaInfinity server (Cloudflare Tunnel to the GCP VM)"
}

# Cloudflare's edge turns away bursts against account creation, sign-in and cloud saves
# before they reach the VM (the server has its own, finer limits behind it). The free plan
# allows one rule, counted over 10 s.
resource "cloudflare_ruleset" "rate_limit" {
  zone_id = data.cloudflare_zone.zone.id
  name    = "ForzaInfinity rate limits"
  kind    = "zone"
  phase   = "http_ratelimit"
  rules = [
    {
      description = "ForzaInfinity: account and save endpoints"
      expression  = "(http.host eq \"${var.hostname}\" and (starts_with(http.request.uri.path, \"/infinity/v1/register\") or starts_with(http.request.uri.path, \"/infinity/v1/login\") or starts_with(http.request.uri.path, \"/infinity/v1/save\") or starts_with(http.request.uri.path, \"/fh2online/v1/\")))"
      action      = "block"
      ratelimit = {
        characteristics     = ["ip.src", "cf.colo.id"]
        period              = 10
        requests_per_period = 20
        mitigation_timeout  = 10
      }
    },
  ]
}

# /infinity/v1/stats (the landing page's numbers) is cached at Cloudflare's edge for as long as
# the server says (Cache-Control: max-age=15): however many visitors the page gets, the VM sees
# about one request per 15 s per Cloudflare location. Nothing else is cached.
resource "cloudflare_ruleset" "cache" {
  zone_id = data.cloudflare_zone.zone.id
  name    = "ForzaInfinity cache"
  kind    = "zone"
  phase   = "http_request_cache_settings"
  rules = [
    {
      description = "ForzaInfinity: public stats"
      expression  = "(http.host eq \"${var.hostname}\" and http.request.method eq \"GET\" and http.request.uri.path eq \"/infinity/v1/stats\")"
      action      = "set_cache_settings"
      action_parameters = {
        cache = true
        edge_ttl = {
          mode = "respect_origin"
        }
        browser_ttl = {
          mode = "respect_origin"
        }
      }
    },
  ]
}
