// SPDX-License-Identifier: AGPL-3.0-or-later
//! mDNS advertise and browse (protocol.md §3.4). Needs multicast on a local
//! interface.

mod common;

use std::time::Duration;

use lull_net::mdns::{Advertisement, Advertiser, Browser, DiscoveryEvent};
use lull_proto::StationTxt;

const WAIT: Duration = Duration::from_secs(15);

async fn next_for(browser: &mut Browser, id: &str) -> DiscoveryEvent {
    loop {
        let ev = tokio::time::timeout(WAIT, browser.next())
            .await
            .expect("no mDNS event in time (is multicast blocked?)")
            .expect("browser stopped");
        let ours = match &ev {
            DiscoveryEvent::Found(s) | DiscoveryEvent::Updated(s) => s.txt.id.as_str() == id,
            DiscoveryEvent::Lost { fullname } => fullname.contains(id),
        };
        if ours {
            return ev;
        }
    }
}

#[tokio::test]
async fn advertise_update_and_withdraw() {
    let id = common::unique_device_id();
    let name = format!("Nursery {id}");
    let txt = StationTxt {
        id: id.clone(),
        pair: false,
        http: Some(47180),
        https: Some(47181),
    };
    let mut ad = Advertiser::start(Advertisement {
        name: name.clone(),
        port: 47100,
        txt: txt.clone(),
    })
    .unwrap();
    let mut browser = Browser::start().unwrap();

    let found = match next_for(&mut browser, id.as_str()).await {
        DiscoveryEvent::Found(s) => s,
        other => panic!("expected Found, got {other:?}"),
    };
    assert_eq!(found.name, name);
    assert_eq!(found.port, 47100);
    assert_eq!(found.txt, txt);
    assert!(!found.addrs.is_empty());
    assert_eq!(found.host, format!("lull-{id}.local."));

    // Opening pairing mode re-announces the TXT record.
    let open = StationTxt {
        pair: true,
        ..txt.clone()
    };
    ad.set_txt(open.clone()).unwrap();
    loop {
        match next_for(&mut browser, id.as_str()).await {
            DiscoveryEvent::Updated(s) if s.txt == open => break,
            DiscoveryEvent::Updated(_) => {}
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    ad.stop().await.unwrap();
    loop {
        match next_for(&mut browser, id.as_str()).await {
            DiscoveryEvent::Lost { fullname } => {
                assert!(fullname.starts_with(&name));
                break;
            }
            // Late resolutions on other interfaces may still arrive.
            DiscoveryEvent::Updated(s) => assert_eq!(s.txt, open),
            other => panic!("expected Lost, got {other:?}"),
        }
    }
}
