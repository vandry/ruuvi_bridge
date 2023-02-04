#include <ArduinoBLE.h>
#include <Arduino_CRC32.h>

Arduino_CRC32 crc32;

struct timer {
  unsigned long next_millis;
  bool force;
  unsigned long interval;
};

struct timer stop_scan_timer = { 0, true, 60000 };
struct timer gpio_timer = { 0, true, 1000 };

bool check_timer(struct timer *t) {
  unsigned long now = millis();
  if (t->force) {
    t->force = false;
  } else {
    if ((t->next_millis < 0x80000000) && (now >= 0x80000000)) {
      return false;
    }
    if (t->next_millis > now) {
      return false;
    }
  }
  t->next_millis = now + t->interval;  // expect 32-bit overflow
  return true;
}

void setup() {
  Serial.begin(9600);
  while (!Serial);
  pinMode(0, INPUT_PULLUP);
  pinMode(1, INPUT_PULLUP);
  pinMode(2, INPUT_PULLUP);
  pinMode(3, INPUT_PULLUP);
  pinMode(4, INPUT_PULLUP);
  pinMode(5, INPUT_PULLUP);
  pinMode(6, INPUT_PULLUP);
  pinMode(7, INPUT_PULLUP);
  if (!BLE.begin()) {
    Serial.println("starting Bluetooth® Low Energy module failed!");
    while (1);
  }
  BLE.scan();
}

void put_message(unsigned char *buf, int dlen) {
  uint32_t const checksum = crc32.calc(buf+4, dlen);
  buf[0] = checksum >> 24;
  buf[1] = (checksum >> 16) & 0xff;
  buf[2] = (checksum >> 8) & 0xff;
  buf[3] = checksum & 0xff;
  Serial.print("{{{");
  int length = dlen + 4;
  for (int i = 0; i < length; i++) {
    unsigned char b = buf[i];
    if (b < 16) {
      Serial.print("0");
    }
    Serial.print(b, HEX);
  }
  Serial.println("}}}");
}

void loop() {
  if (check_timer(&stop_scan_timer)) {
    Serial.println("stop scan");
    BLE.stopScan();
    delay(1000);
    Serial.println("restart scan");
    BLE.scan();
  }

  if (check_timer(&gpio_timer)) {
    uint8_t buf[9];
    buf[4] = 'G';
    buf[5] = 'P';
    buf[6] = 'I';
    buf[7] = 'O';
    buf[8] =
        (digitalRead(0) == HIGH ? 1 : 0) |
        (digitalRead(1) == HIGH ? 2 : 0) |
        (digitalRead(2) == HIGH ? 4 : 0) |
        (digitalRead(3) == HIGH ? 8 : 0) |
        (digitalRead(4) == HIGH ? 16 : 0) |
        (digitalRead(5) == HIGH ? 32 : 0) |
        (digitalRead(6) == HIGH ? 64 : 0) |
        (digitalRead(7) == HIGH ? 128 : 0);
    put_message(buf, 5);
  }

  BLEDevice peripheral = BLE.available();
  if (peripheral) {
    if (peripheral.hasManufacturerData()) {
      int dlen = peripheral.manufacturerDataLength();
      int len_with_crc32 = dlen + 4;
      uint8_t buf[len_with_crc32];
      if (peripheral.manufacturerData(buf+4, dlen)) {
        if ((dlen > 2) && (buf[4] == 0x99) && (buf[5] == 0x04) && (buf[6] == 5)) {
          put_message(buf, dlen);
        }
      }
    }
  }
}
