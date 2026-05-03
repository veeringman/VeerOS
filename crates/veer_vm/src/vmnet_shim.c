#include <dispatch/dispatch.h>
#include <stdint.h>
#include <string.h>
#include <sys/uio.h>
#include <vmnet/vmnet.h>
#include <xpc/xpc.h>

// mode: 0 = bridged, 1 = host, 2 = shared
// bridge_if: physical interface name for bridged mode (e.g. "en0"); NULL for shared/host
int veer_vmnet_start(uint32_t mode, const char *mac, const char *bridge_if, void **out_interface) {
    if (out_interface == NULL) {
        return VMNET_INVALID_ARGUMENT;
    }
    *out_interface = NULL;

    xpc_object_t desc = xpc_dictionary_create(NULL, NULL, 0);
    if (desc == NULL) {
        return VMNET_MEM_FAILURE;
    }

    uint64_t vmnet_mode;
    if (mode == 0) {
        vmnet_mode = VMNET_BRIDGED_MODE;
    } else if (mode == 1) {
        vmnet_mode = VMNET_HOST_MODE;
    } else {
        vmnet_mode = VMNET_SHARED_MODE;
    }
    xpc_dictionary_set_uint64(desc, vmnet_operation_mode_key, vmnet_mode);
    xpc_dictionary_set_uint64(desc, vmnet_mtu_key, 1500);
    xpc_dictionary_set_uint64(desc, vmnet_max_packet_size_key, 1514);
    xpc_dictionary_set_bool(desc, vmnet_allocate_mac_address_key, false);
    if (mac != NULL && mac[0] != '\0') {
        xpc_dictionary_set_string(desc, vmnet_mac_address_key, mac);
    }
    if (mode == 0 && bridge_if != NULL && bridge_if[0] != '\0') {
        xpc_dictionary_set_string(desc, vmnet_shared_interface_name_key, bridge_if);
    }

    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block vmnet_return_t completion_status = VMNET_FAILURE;
    interface_ref iface = vmnet_start_interface(
        desc,
        dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0),
        ^(vmnet_return_t status, xpc_object_t params) {
            (void)params;
            completion_status = status;
            dispatch_semaphore_signal(sem);
        });

    if (iface == NULL) {
        xpc_release(desc);
        return VMNET_FAILURE;
    }

    dispatch_semaphore_wait(sem, DISPATCH_TIME_FOREVER);
    xpc_release(desc);

    if (completion_status != VMNET_SUCCESS) {
        return completion_status;
    }

    *out_interface = iface;
    return VMNET_SUCCESS;
}

int veer_vmnet_stop(void *interface) {
    if (interface == NULL) {
        return VMNET_SUCCESS;
    }

    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block vmnet_return_t completion_status = VMNET_FAILURE;
    vmnet_return_t rc = vmnet_stop_interface(
        (interface_ref)interface,
        dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0),
        ^(vmnet_return_t status) {
            completion_status = status;
            dispatch_semaphore_signal(sem);
        });

    if (rc != VMNET_SUCCESS) {
        return rc;
    }
    dispatch_semaphore_wait(sem, DISPATCH_TIME_FOREVER);
    return completion_status;
}

int veer_vmnet_write_frame(void *interface, const uint8_t *frame, size_t len) {
    if (interface == NULL || frame == NULL) {
        return VMNET_INVALID_ARGUMENT;
    }

    struct iovec iov = {
        .iov_base = (void *)frame,
        .iov_len = len,
    };
    struct vmpktdesc pkt = {
        .vm_pkt_size = len,
        .vm_pkt_iov = &iov,
        .vm_pkt_iovcnt = 1,
        .vm_flags = 0,
    };
    int count = 1;
    return vmnet_write((interface_ref)interface, &pkt, &count);
}

int veer_vmnet_read_frame(void *interface, uint8_t *frame, size_t cap, size_t *out_len) {
    if (interface == NULL || frame == NULL || out_len == NULL) {
        return VMNET_INVALID_ARGUMENT;
    }
    *out_len = 0;

    struct iovec iov = {
        .iov_base = frame,
        .iov_len = cap,
    };
    struct vmpktdesc pkt = {
        .vm_pkt_size = cap,
        .vm_pkt_iov = &iov,
        .vm_pkt_iovcnt = 1,
        .vm_flags = 0,
    };
    int count = 1;
    vmnet_return_t rc = vmnet_read((interface_ref)interface, &pkt, &count);
    if (rc == VMNET_SUCCESS && count > 0) {
        *out_len = pkt.vm_pkt_size;
    }
    return rc;
}