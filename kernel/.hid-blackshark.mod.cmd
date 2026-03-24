savedcmd_hid-blackshark.mod := printf '%s\n'   hid-blackshark.o | awk '!x[$$0]++ { print("./"$$0) }' > hid-blackshark.mod
