// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * HID driver for the Razer BlackShark V3 Pro wireless headset.
 *
 * Exposes battery level and charging status via the power_supply subsystem
 * so UPower and KDE/GNOME battery widgets pick it up automatically.
 *
 * Protocol reverse engineered via usbmon capture with Razer Synapse on
 * Windows 11. No vendor documentation was used.
 *
 * Hardware tested: Razer BlackShark V3 Pro (USB 1532:0577)
 *
 * TODO: mic mute button -> KEY_MICMUTE input event (needs HID report capture)
 */

#include <linux/hid.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/power_supply.h>
#include <linux/workqueue.h>
#include <linux/completion.h>

#define USB_VENDOR_ID_RAZER                  0x1532
#define USB_DEVICE_ID_RAZER_BLACKSHARK_V3_PRO 0x0577

/* HID report format (64 bytes, confirmed from usbmon) */
#define BLACKSHARK_REPORT_LEN   64
#define BLACKSHARK_REPORT_ID    0x02
#define BLACKSHARK_TRANSACTION  0x60  /* constant — matches Synapse */

/* Byte offsets within the report */
#define REP_STATUS   1
#define REP_TX       2
#define REP_DATASIZ  6
#define REP_FLAGS    9
#define REP_CLASS    10
#define REP_SUB      11
#define REP_CMD      12
#define REP_ARGS     13
#define REP_CRC      62

/* Response status byte */
#define STATUS_OK    0x02

/* Battery command: class=0x21, cmd=0x00, args=[0x00] */
#define BATTERY_CLASS  0x21
#define BATTERY_CMD    0x00

/* Charging state.
 * GET: class=0x2a, cmd=0x00, args=[0x00] — query current state at startup.
 * Unsolicited push: sub=0x02, class=0x2a, cmd=0x01, args[0]=1/0 on change. */
#define CHARGING_CLASS    0x2a
#define CHARGING_GET_CMD  0x00
#define CHARGING_NOTIF_CMD 0x01
#define NOTIF_SUB         0x02

/* Battery poll interval: 5 minutes */
#define BATTERY_POLL_INTERVAL (5 * 60 * HZ)

/* Timeout waiting for a response from the headset (ms) */
#define RESPONSE_TIMEOUT_MS 2000

struct blackshark_data {
	struct hid_device        *hdev;

	/* power_supply */
	struct power_supply      *battery;
	struct power_supply_desc  battery_desc;
	char                      battery_name[32];

	/* Periodic battery polling */
	struct delayed_work       battery_work;

	/* Synchronisation between request sender and raw_event receiver */
	struct mutex              req_lock;   /* serialise HID requests */
	struct completion         response;
	u8                        resp_buf[BLACKSHARK_REPORT_LEN];
	u8                        cmd_buf[BLACKSHARK_REPORT_LEN]; /* heap, DMA-safe */

	/* Cached state (updated under req_lock or from battery_work) */
	u8   capacity;         /* 0–100 */
	bool charging;
	bool charging_known;   /* false until first cls=0x2a notification or query */
	bool initial_done;     /* true after first battery_work has run */
	bool present;          /* headset is wirelessly connected to dongle */
};

/* ---------------------------------------------------------------------------
 * Report builder
 * ------------------------------------------------------------------------- */

static u8 blackshark_crc(const u8 *buf)
{
	u8 crc = 0;
	int i;

	for (i = 0; i < REP_CRC; i++)
		crc ^= buf[i];
	return crc;
}

static void blackshark_build_report_flags(u8 *buf, u8 flags, u8 class, u8 cmd,
					   const u8 *args, size_t args_len)
{
	memset(buf, 0, BLACKSHARK_REPORT_LEN);
	buf[0]           = BLACKSHARK_REPORT_ID;
	buf[REP_TX]      = BLACKSHARK_TRANSACTION;
	buf[REP_FLAGS]   = flags;
	buf[REP_DATASIZ] = 3 + args_len; /* class + sub + cmd + args */
	buf[REP_CLASS]   = class;
	buf[REP_SUB]     = 0x00;
	buf[REP_CMD]     = cmd;
	if (args && args_len)
		memcpy(&buf[REP_ARGS], args, args_len);
	buf[REP_CRC] = blackshark_crc(buf);
}

static void blackshark_build_report(u8 *buf, u8 class, u8 cmd,
				     const u8 *args, size_t args_len)
{
	blackshark_build_report_flags(buf, 0x80, class, cmd, args, args_len);
}

/* ---------------------------------------------------------------------------
 * HID request / response
 * ------------------------------------------------------------------------- */

/*
 * Send a report and wait for the device to echo it back with status=OK.
 * Must be called with req_lock held.
 */
static int blackshark_request_flags(struct blackshark_data *data,
				     u8 flags, u8 class, u8 cmd,
				     const u8 *args, size_t args_len)
{
	int ret;
	long wait;

	blackshark_build_report_flags(data->cmd_buf, flags, class, cmd,
				      args, args_len);

	reinit_completion(&data->response);

	ret = hid_hw_raw_request(data->hdev, BLACKSHARK_REPORT_ID,
				 data->cmd_buf, BLACKSHARK_REPORT_LEN,
				 HID_OUTPUT_REPORT, HID_REQ_SET_REPORT);
	if (ret < 0) {
		hid_err(data->hdev, "raw_request failed: %d\n", ret);
		return ret;
	}

	wait = wait_for_completion_timeout(&data->response,
					   msecs_to_jiffies(RESPONSE_TIMEOUT_MS));
	if (!wait)
		return -ETIMEDOUT;

	if (data->resp_buf[REP_STATUS] != STATUS_OK)
		return -EIO;

	return 0;
}

static int blackshark_request(struct blackshark_data *data,
			       u8 class, u8 cmd,
			       const u8 *args, size_t args_len)
{
	int ret;
	long wait;

	/* cmd_buf lives in the struct (heap), not on the stack — required for DMA */
	blackshark_build_report(data->cmd_buf, class, cmd, args, args_len);

	reinit_completion(&data->response);

	ret = hid_hw_raw_request(data->hdev, BLACKSHARK_REPORT_ID,
				 data->cmd_buf, BLACKSHARK_REPORT_LEN,
				 HID_OUTPUT_REPORT, HID_REQ_SET_REPORT);
	if (ret < 0) {
		hid_err(data->hdev, "raw_request failed: %d\n", ret);
		return ret;
	}

	wait = wait_for_completion_timeout(&data->response,
					   msecs_to_jiffies(RESPONSE_TIMEOUT_MS));
	if (!wait) {
		hid_warn(data->hdev, "response timeout (class=0x%02x cmd=0x%02x)\n",
			 class, cmd);
		return -ETIMEDOUT;
	}

	if (data->resp_buf[REP_STATUS] != STATUS_OK) {
		hid_warn(data->hdev, "bad response status: 0x%02x\n",
			 data->resp_buf[REP_STATUS]);
		return -EIO;
	}

	return 0;
}

/* ---------------------------------------------------------------------------
 * Battery
 * ------------------------------------------------------------------------- */

static int blackshark_query_battery(struct blackshark_data *data)
{
	const u8 args[] = { 0x00 };
	int ret;

	mutex_lock(&data->req_lock);
	ret = blackshark_request(data, BATTERY_CLASS, BATTERY_CMD,
				 args, sizeof(args));
	if (!ret) {
		/* Response args: [0]=percentage. Charging state is tracked
		 * separately via blackshark_query_charging() and unsolicited
		 * cls=0x2a notifications — do NOT overwrite it here. */
		data->capacity = data->resp_buf[REP_ARGS];
		data->present  = true;
		hid_info(data->hdev, "battery: %u%% charging=%d\n",
			 data->capacity, data->charging);
	} else {
		/* Headset is off or out of range */
		data->present = false;
	}
	mutex_unlock(&data->req_lock);

	if (data->battery)
		power_supply_changed(data->battery);

	return ret;
}

/*
 * Replicate the two-command init sequence Synapse sends before querying
 * charging state. Without this, the device ignores the cls=0x2a GET.
 *
 * Observed order in Synapse startup capture:
 *   1. cls=0x02, flag=0x00  (device capability init)
 *   2. cls=0x2a, flag=0x00  (capability query — response ignored)
 *   3. cls=0x2a, flag=0x80  (charging state GET — response has charging flag)
 */
static int blackshark_query_charging(struct blackshark_data *data)
{
	const u8 args[] = { 0x00 };
	int ret;

	mutex_lock(&data->req_lock);

	/* Step 1: device init command (flag=0x00) — Synapse sends this first */
	blackshark_request_flags(data, 0x00, 0x02, 0x00, args, sizeof(args));

	/* Step 2: capability query (flag=0x00) — response not needed */
	blackshark_request_flags(data, 0x00, CHARGING_CLASS, CHARGING_GET_CMD,
				 args, sizeof(args));

	/* Step 3: actual charging state GET (flag=0x80) */
	ret = blackshark_request(data, CHARGING_CLASS, CHARGING_GET_CMD,
				 args, sizeof(args));
	if (!ret) {
		data->charging       = data->resp_buf[REP_ARGS] != 0;
		data->charging_known = true;
		hid_info(data->hdev, "initial charging state: %s\n",
			 data->charging ? "charging" : "discharging");
	} else {
		hid_warn(data->hdev, "charging query failed: %d\n", ret);
	}

	mutex_unlock(&data->req_lock);
	return ret;
}

static void blackshark_battery_work(struct work_struct *work)
{
	struct blackshark_data *data =
		container_of(work, struct blackshark_data, battery_work.work);

	/*
	 * On first run the device is ready (it responds to battery queries)
	 * so we also query initial charging state here. We can't do this in
	 * probe because the device doesn't respond during that early window.
	 */
	if (!data->initial_done) {
		data->initial_done = true;
		if (!data->charging_known)
			blackshark_query_charging(data);
	}

	blackshark_query_battery(data);
	schedule_delayed_work(&data->battery_work, BATTERY_POLL_INTERVAL);
}

/* power_supply callbacks */

static int blackshark_ps_get_property(struct power_supply *psy,
				       enum power_supply_property psp,
				       union power_supply_propval *val)
{
	struct blackshark_data *data = power_supply_get_drvdata(psy);

	switch (psp) {
	case POWER_SUPPLY_PROP_STATUS:
		if (!data->present || !data->charging_known)
			val->intval = POWER_SUPPLY_STATUS_UNKNOWN;
		else if (data->charging)
			val->intval = POWER_SUPPLY_STATUS_CHARGING;
		else
			val->intval = POWER_SUPPLY_STATUS_DISCHARGING;
		break;
	case POWER_SUPPLY_PROP_CAPACITY:
		val->intval = data->present ? data->capacity : 0;
		break;
	case POWER_SUPPLY_PROP_PRESENT:
		val->intval = data->present ? 1 : 0;
		break;
	case POWER_SUPPLY_PROP_SCOPE:
		val->intval = POWER_SUPPLY_SCOPE_DEVICE;
		break;
	default:
		return -EINVAL;
	}
	return 0;
}

static enum power_supply_property blackshark_ps_props[] = {
	POWER_SUPPLY_PROP_STATUS,
	POWER_SUPPLY_PROP_CAPACITY,
	POWER_SUPPLY_PROP_PRESENT,
	POWER_SUPPLY_PROP_SCOPE,
};

/* ---------------------------------------------------------------------------
 * HID driver callbacks
 * ------------------------------------------------------------------------- */

static int blackshark_raw_event(struct hid_device *hdev,
				struct hid_report *report,
				u8 *data, int size)
{
	struct blackshark_data *d = hid_get_drvdata(hdev);

	if (size != BLACKSHARK_REPORT_LEN || data[0] != BLACKSHARK_REPORT_ID)
		return 0;

	/* Is this a response to one of our requests? */
	if (data[REP_SUB] == 0x01) {
		memcpy(d->resp_buf, data, BLACKSHARK_REPORT_LEN);
		complete(&d->response);
		/* Don't pass battery responses to hidraw — they're ours */
		if (data[REP_CLASS] == BATTERY_CLASS)
			return 1;
	} else if (data[REP_SUB] == NOTIF_SUB) {
		/* Unsolicited notification from the headset */
		if (data[REP_CLASS] == CHARGING_CLASS &&
		    data[REP_CMD]   == CHARGING_NOTIF_CMD) {
			d->charging       = data[REP_ARGS] != 0;
			d->charging_known = true;
			hid_info(hdev, "charging state: %s\n",
				 d->charging ? "charging" : "discharging");
			if (d->battery)
				power_supply_changed(d->battery);
			return 1;
		}
		/* TODO: mute button and other notifications go here */
	}

	/* TODO: mute button input event
	 * When the mute button is pressed the headset sends an unsolicited
	 * HID report. Once we capture the report format, emit KEY_MICMUTE
	 * via an input device registered here.
	 */

	return 0; /* pass through to hidraw */
}

static int blackshark_probe(struct hid_device *hdev,
			     const struct hid_device_id *id)
{
	struct blackshark_data *data;
	struct power_supply_config ps_cfg = {};
	int ret;

	data = devm_kzalloc(&hdev->dev, sizeof(*data), GFP_KERNEL);
	if (!data)
		return -ENOMEM;

	data->hdev = hdev;
	mutex_init(&data->req_lock);
	init_completion(&data->response);
	INIT_DELAYED_WORK(&data->battery_work, blackshark_battery_work);

	hid_set_drvdata(hdev, data);

	ret = hid_parse(hdev);
	if (ret) {
		hid_err(hdev, "hid_parse failed: %d\n", ret);
		return ret;
	}

	/*
	 * HID_CONNECT_DEFAULT keeps the hidraw interface open so
	 * blacksharkd can still send sidetone/EQ commands via /dev/hidraw*.
	 */
	ret = hid_hw_start(hdev, HID_CONNECT_DEFAULT);
	if (ret) {
		hid_err(hdev, "hid_hw_start failed: %d\n", ret);
		return ret;
	}

	ret = hid_hw_open(hdev);
	if (ret) {
		hid_err(hdev, "hid_hw_open failed: %d\n", ret);
		goto err_stop;
	}

	/* Register battery with power_supply */
	snprintf(data->battery_name, sizeof(data->battery_name),
		 "blackshark-headset");
	data->battery_desc.name             = data->battery_name;
	data->battery_desc.type             = POWER_SUPPLY_TYPE_BATTERY;
	data->battery_desc.properties       = blackshark_ps_props;
	data->battery_desc.num_properties   = ARRAY_SIZE(blackshark_ps_props);
	data->battery_desc.get_property     = blackshark_ps_get_property;

	ps_cfg.drv_data = data;

	data->battery = devm_power_supply_register(&hdev->dev,
						    &data->battery_desc,
						    &ps_cfg);
	if (IS_ERR(data->battery)) {
		ret = PTR_ERR(data->battery);
		hid_err(hdev, "power_supply_register failed: %d\n", ret);
		goto err_close;
	}

	/* Charging state queried on first battery_work tick (device isn't
	 * ready to respond during probe). */
	schedule_delayed_work(&data->battery_work, HZ);

	hid_info(hdev, "Razer BlackShark V3 Pro connected\n");
	return 0;

err_close:
	hid_hw_close(hdev);
err_stop:
	hid_hw_stop(hdev);
	return ret;
}

static void blackshark_remove(struct hid_device *hdev)
{
	struct blackshark_data *data = hid_get_drvdata(hdev);

	cancel_delayed_work_sync(&data->battery_work);
	hid_hw_close(hdev);
	hid_hw_stop(hdev);
	hid_info(hdev, "Razer BlackShark V3 Pro disconnected\n");
}

/* ---------------------------------------------------------------------------
 * Module boilerplate
 * ------------------------------------------------------------------------- */

static const struct hid_device_id blackshark_devices[] = {
	{ HID_USB_DEVICE(USB_VENDOR_ID_RAZER,
			 USB_DEVICE_ID_RAZER_BLACKSHARK_V3_PRO) },
	{ }
};
MODULE_DEVICE_TABLE(hid, blackshark_devices);

static struct hid_driver blackshark_driver = {
	.name       = "hid-blackshark",
	.id_table   = blackshark_devices,
	.probe      = blackshark_probe,
	.remove     = blackshark_remove,
	.raw_event  = blackshark_raw_event,
};
module_hid_driver(blackshark_driver);

MODULE_AUTHOR("Matt Smith");
MODULE_DESCRIPTION("HID driver for Razer BlackShark V3 Pro headset");
MODULE_LICENSE("GPL");
