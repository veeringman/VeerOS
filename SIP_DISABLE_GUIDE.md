# Disabling System Integrity Protection (SIP) on macOS

**Target Mac**: MacBook Pro 12,1 (2015) / macOS 12.6.4 (Monterey)
**Goal**: Disable SIP to allow HVF exclusive hypervisor access for veer_vm

---

## ⚠️ WARNING

Disabling SIP reduces macOS security. Only do this on a trusted, isolated development machine. Re-enable SIP after testing if security is a concern.

---

## Step 1: Preparation

1. **Back up important data** (optional but recommended)
2. **Close all applications** on the Mac
3. **Ensure power is connected** (don't let battery die during reboot)

---

## Step 2: Boot into Recovery Mode

### On Intel Mac (your MacBook Pro):

1. **Shut down the Mac** completely
2. **Power on and immediately hold ⌘ + R** (Command + R)
   - Keep holding until you see the Apple logo or "macOS Utilities" screen
   - This boots into Recovery Mode
3. **Wait for "macOS Utilities" window** to appear
4. You should see the macOS version (Monterey) and various utility options

---

## Step 3: Open Terminal in Recovery Mode

1. In the **macOS Utilities** window, click **Utilities** menu (top menu bar)
2. Select **Terminal**
3. A Terminal window opens

---

## Step 4: Disable SIP

In the Terminal, run:

```bash
csrutil disable
```

You may be prompted for:
- Your Mac password (user: `vijay`, pass: `vijay42o`)
- Or it may proceed without prompt in Recovery Mode

**Expected output:**
```
System Integrity Protection status: enabled.
To disable System Integrity Protection, an administrator must authenticate.
Password:
(enter password if prompted)

System Integrity Protection has been successfully disabled. 
Please restart the machine for the changes to take effect.
```

---

## Step 5: Restart

Type in Terminal:
```bash
reboot
```

Or use menu: **Apple menu → Restart**

The Mac will restart. **Do not hold any key combination** on reboot—let it boot normally.

---

## Step 6: Verify SIP is Disabled

Once the Mac boots back to normal desktop, open Terminal and verify:

```bash
csrutil status
```

**Expected output:**
```
System Integrity Protection status: disabled.
```

✓ **SIP is now disabled**

---

## Step 7: Test veer_vm HVF Boot

From the Linux machine (alien), SSH to the Mac and test:

```bash
sshpass -p 'vijay42o' ssh -o StrictHostKeyChecking=no vijay@192.168.29.74 \
  'cd ~/rnd/VeerOS && cargo run -q -p veer_vm --target x86_64-apple-darwin -- \
  --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
  --memory 128 --hvf-run-once --hvf-inject-timer'
```

**Expected outcomes:**

- **Success**: Boot output, then "Welcome to VeerOS" banner, clean exit (0)
- **Still fails with -85377017**: SIP wasn't the issue; hardware/firmware limitation likely
- **Different error**: Indicates progress; report to proceed with debugging

---

## Step 8: Re-enable SIP (Optional, Recommended for Security)

If you want to restore security after testing:

1. **Boot back into Recovery Mode** (⌘ + R at startup)
2. **Open Terminal** from Utilities menu
3. Run:
   ```bash
   csrutil enable
   ```
4. **Reboot normally**

Verify:
```bash
csrutil status
```

---

## Troubleshooting

### Recovery Mode doesn't boot
- Try holding **⌘ + Option + R** (for Internet Recovery) instead
- Or **⌘ + Shift + Option + R** (for latest macOS Recovery)

### Terminal not available in Recovery Mode
- Try clicking **Utilities** menu (may need to click in the window first)
- If Utilities menu doesn't appear, try **Window** menu → **Utilities**

### "System Integrity Protection has already been disabled"
- SIP is already off from a previous attempt; proceed to Step 7

### csrutil command not found
- Make sure you're in the Recovery Mode Terminal (not normal Terminal)
- Recovery Mode Terminal should show "Recovery" in the title

---

## Success Checklist

- [ ] Boot into Recovery Mode successfully
- [ ] Run `csrutil disable` in Terminal
- [ ] See confirmation message
- [ ] Reboot normally
- [ ] Verify `csrutil status` shows "disabled"
- [ ] Test veer_vm HVF boot
- [ ] Document result (success or error details)

---

## Next Steps After SIP Disable

If HVF now works:
- **C1-C6**: Test veer_vm boot to banner on HVF
- **D1-D3**: Implement user-space LAPIC + timer
- **E1-E4**: Add UART, virtio-blk, virtio-net, snapshots

If HVF still fails:
- Hardware/firmware limitation confirmed
- Focus on Linux KVM testing instead
- D/E/F phases can proceed on KVM
