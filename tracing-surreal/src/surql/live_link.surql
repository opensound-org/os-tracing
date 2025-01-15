LET $table = type::table($table_name);
LIVE SELECT *, client_id[*] FROM $table;
