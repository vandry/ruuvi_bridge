ruuvi_bridge
============

Serves data reports from https://ruuvi.com/ruuvitag/ sensors as
metrics for Prometheus.

```
Ruuvitag ---BLE--\
                  \
Ruuvitag ---BLE--\ \
                  X-X--->  Arduino  --USB-serial-->  ruuvi_bridge
Ruuvitag ---BLE--/ /                           http://foo:port/metrics
                  /                                       |
Ruuvitag ---BLE--/                     Prometheus --------/
```

Why the complicated 2-step system with an Arduino plus a regular
computer? Because the Arduino is the only thing I happened to have
that was Bluetooth-equipped and that could be commissioned and
dedicated to this purpose. Besides, as soon as I have the necesary
parts I plan to use the GPIOs on the Arduino to sense whether or not
the heating is active in the various rooms where the Ruuvitags are
deployed (which is the thing I actually wanted in the first place).

Why not serve the /metrics directly on the Arduino? I'm displeased with
the Arduino's wifi support. It seems to have no support at all for IPv6.
AFAICT, all the packet processing is offboarded to a separate chip so
it's not even possible to fix this by writing more code for the Arduino.
I'm not going to go play in the DHCP server to arrange a static IPv4
address and then arranging to scrape over that like some kind of
denizen of the 20th century. Anyway the Arduino needed power anyway,
connecting it via USB solves that, and then it's only a step further
to use the USB for data transfer too.
