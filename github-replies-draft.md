# Draft replies (not posted)

## #148 Pairing fails with Hyper-V adapters (samsarajr)

Thank you, the packet capture made this one quick to pin down. You were right on both counts:

- The pairing code carries three of the PC's addresses, and all private addresses ranked the same, so your three Hyper-V adapters took every slot and the Wi-Fi address was left out. The phone never had 192.168.1.123 to try.
- The PC listened for nearby phones only on the adapter Windows picks for multicast, which on your PC is a Hyper-V one (metric 15), so it never answered the phone's queries on Wi-Fi.

Both are fixed for the next release: the code now lists the address the PC uses to reach your network first, and the PC looks for phones on all of its networks (except Tailscale's, which carries no multicast). Until then, disabling the vEthernet adapters as you did is the way around it.

## #141 Can not connect at all (Mullvad on the PC)

A possible cause turned up in #148: on a PC with extra network adapters (Hyper-V, WSL, Docker, or a VPN like Mullvad), the PC could leave its real Wi-Fi or Ethernet address out of the pairing code and listen for the phone on the wrong adapter. That's fixed for the next release. When it's out, could you try pairing again, with Mullvad on and with it off?

## D#150 Couldn't reach the PC (Redmi Note 13 Pro)

Does your laptop have Hyper-V, WSL, Docker, VirtualBox or a VPN installed? Any of them adds network adapters, and the current version can then put the wrong addresses in the pairing code (#148). The next release fixes that. Until then, these are worth a try:

- On the PC, **Settings › Connection & About › Connection Doctor** says what it can see.
- Check that Windows treats your Wi-Fi as a **Private** network (Windows Settings › Network & internet › Wi-Fi › your network).
- Some routers keep devices on the same Wi-Fi apart (often guest networks). A USB cable works around that.

## D#151 校园网怎么连 (Chinese)

校园网通常开启了"客户端隔离"（AP 隔离）：同一个 Wi-Fi 下的设备彼此看不到，所以手机和电脑无法直接连接。可以这样解决：

- 在两台设备上都打开**通过移动数据连接**：手机在 **设置 › 设备 › 通过移动数据连接**，电脑在 **Settings › Connection & About › Connect over mobile data**。打开后，直接连不上时会经过加密的中继连接，校园网和 VPN 下一般也能用。
- 用 USB 数据线连接，也不受网络限制。
- Tailscale：下一个版本支持通过 Tailscale 的地址直接连接（不需要中继）。

开 VPN 就断开，是因为 VPN 把局域网的流量也送进了隧道。在 VPN 客户端里打开"允许局域网访问"或分应用代理（分流），或者像上面一样打开**通过移动数据连接**。

## D#149 microSD Access

You didn't miss anything: 0.1.4 shows only the phone's internal storage. The next release adds it: with **All files access**, the SD card shows as a folder of its own (named as your phone names it, usually **SD card**) beside the internal storage's folders, and you can open, add, rename and delete files there as elsewhere. On Android 10 and older, Android lets apps only read SD cards. Until then, with **Selected folders only** you can pick a folder on the SD card in Android's picker.

## #152 Open apps while the phone's screen is off

Thanks, this was easy to reproduce once I knew the screen had to be off. An app window runs on a display of its own, and Android starts that display asleep while the phone is asleep, so the app never drew anything. The next release wakes the phone's screen when an app window opens, as mirroring already does.

## #108 (aarya-soul)

Thanks for taking the links down and for the clean install test, that's exactly what helps. Good to know the AppImage runs without extract-and-run and the .deb installs cleanly on 24.04.

## Upstream: n0-computer/iroh-address-lookups#7 (comment)

Hi @cbenhagen, @flub. We hit this in NectarLink, an app built on iroh: pairing failed on Windows PCs whose Hyper-V adapters rank above Wi-Fi, because the mDNS socket only joined the group on a vEthernet adapter (thtbee/nectarlink#148 has a pktmon capture). We carry a small patch until this lands, so I looked into why CI is red here.

The failures reproduce in a Linux network namespace, and they come down to a second IPv4 interface that's up. GitHub's Ubuntu runners have `docker0`. Each test ran in its own process, as nextest's isolation group does:

| Interfaces | main | this PR |
|---|---|---|
| `lo`, `eth0` | 12/12 | 16/16 (also with swarm-discovery 0.6.5) |
| plus a second interface, up | 12/12 | `mdns_subscribe` fails every run |
| plus the same interface, down | | 16/16 |

With two interfaces every query and response goes out once per interface. In `mdns_subscribe`, all five endpoints answer within the 5 s on main; with this PR one of them never gets a turn, and in one run the subscriber also got an `Expired` for a live peer, which ends the test early too.

To reproduce (as root, then run the test binary inside the namespace as your user):

```sh
ip netns add mdns
ip link add vm0 type veth peer name eth0 netns mdns
ip link add vm1 type veth peer name eth1 netns mdns
ip link set vm0 up; ip link set vm1 up
ip netns exec mdns sh -c 'ip link set lo up
  ip addr add 10.1.0.2/24 dev eth0; ip link set eth0 up; ip route add default via 10.1.0.1
  ip addr add 172.18.0.2/16 dev eth1; ip link set eth1 up'
ip netns exec mdns sudo -u "$SUDO_USER" ./target/debug/deps/iroh_mdns_address_lookup-<hash> \
  --exact tests::run_in_isolation::mdns_subscribe
```

One more thing from our side, since this turns multicast on for every interface: on Android that includes mobile data, where it keeps the cellular radio awake, so our phones stay on the single default socket. Would a way to narrow the set (a predicate, or opting out) fit in this PR?

Happy to help with the test or the rebase if that's useful.
