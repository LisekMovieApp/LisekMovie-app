//! Webhook transport with bounded retries and DNS pinning.
use std::{net::IpAddr,time::Duration};

pub fn retry_delay(attempt:u32)->u64{5u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(9))}
fn allow_local()->bool{std::env::var("LISEK_ALLOW_LOCAL_WEBHOOKS").as_deref()==Ok("1")}
pub fn public_ip(ip:IpAddr)->bool{
    match ip{
        IpAddr::V4(ip)=>{
            let o=ip.octets();
            !(ip.is_private()||ip.is_loopback()||ip.is_link_local()||ip.is_unspecified()||ip.is_broadcast()||ip.is_multicast()||ip.is_documentation()||o[0]==0||o[0]>=240||(o[0]==100&&(64..=127).contains(&o[1]))||(o[0]==198&&(18..=19).contains(&o[1]))||(o[0]==192&&o[1]==0))
        },
        IpAddr::V6(ip)=>{
            if let Some(v4)=ip.to_ipv4_mapped(){return public_ip(IpAddr::V4(v4));}
            let s=ip.segments();
            // Limit to global unicast and exclude documentation and transition prefixes.
            (s[0]&0xe000)==0x2000 && s[0]!=0x2002 && !(s[0]==0x2001&&(s[1]<0x0200||s[1]==0x0db8))
        }
    }
}
pub fn validate_url(value:&str)->Result<reqwest::Url,&'static str>{
    let url=reqwest::Url::parse(value).map_err(|_|"invalid_url")?;
    if url.scheme()!="https" && !(allow_local()&&url.scheme()=="http"){return Err("https_required");}
    if !url.username().is_empty()||url.password().is_some()||url.fragment().is_some(){return Err("invalid_url");}
    let host=url.host_str().ok_or("invalid_host")?.trim_matches(['[',']']);
    if !allow_local() && (host.eq_ignore_ascii_case("localhost")||host.parse::<IpAddr>().is_ok_and(|ip|!public_ip(ip))){return Err("private_address");}
    Ok(url)
}
pub async fn deliver(value:&str,destination:&str,payload:&serde_json::Value)->Result<u16,&'static str>{
    let url=validate_url(value)?;
    let host=url.host_str().ok_or("invalid_host")?.trim_matches(['[',']']).to_owned();
    let port=url.port_or_known_default().ok_or("invalid_port")?;
    let addresses=tokio::time::timeout(Duration::from_secs(5),tokio::net::lookup_host((host.as_str(),port))).await.map_err(|_|"dns_timeout")?.map_err(|_|"dns_failed")?.collect::<Vec<_>>();
    if addresses.is_empty() || (!allow_local()&&addresses.iter().any(|a|!public_ip(a.ip()))){return Err("private_address");}
    let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(10)).resolve_to_addrs(&host,&addresses).build().map_err(|_|"client_failed")?;
    let title=payload.get("title").and_then(|v|v.as_str()).unwrap_or("New media");
    let body=match destination{
        "discord"=>serde_json::json!({"content":title,"allowed_mentions":{"parse":[]}}),
        "slack"=>serde_json::json!({"text":title,"mrkdwn":false}),
        _=>payload.clone()
    };
    client.post(url).json(&body).send().await.map(|r|r.status().as_u16()).map_err(|_|"delivery_failed")
}
#[cfg(test)]
mod tests{
    #[test]
    fn retry_backoff_grows_and_is_bounded(){
        assert_eq!(super::retry_delay(1),5);
        assert_eq!(super::retry_delay(2),10);
        assert_eq!(super::retry_delay(5),80);
        assert_eq!(super::retry_delay(u32::MAX),2560);
    }
    #[test]
    fn private_and_metadata_networks_are_rejected(){
        for ip in ["127.0.0.1","10.0.0.1","169.254.169.254","100.64.0.1","::1","::ffff:127.0.0.1","fc00::1","2002:7f00:1::"]{assert!(!super::public_ip(ip.parse().unwrap()),"{ip}");}
        assert!(super::public_ip("8.8.8.8".parse().unwrap()));
    }
}

