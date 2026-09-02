#!/bin/sh
printf '>Fixture Dynamic Page\nremote=%s\nlink=%s\nfield_name=%s\nvar_mode=%s\n' "${remote_identity:-none}" "${link_id:-none}" "${field_name:-none}" "${var_mode:-none}"
