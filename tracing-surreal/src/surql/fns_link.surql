DEFINE FUNCTION OVERWRITE fn::check_n($n: int) {
	IF $n <= 0 OR $n > 65535
		{
			THROW 'n must be in the range: [1, 65535]';
		}
	;
}
	PERMISSIONS FULL
;
DEFINE FUNCTION OVERWRITE fn::all_desc_client($table_name: string) {
	RETURN (SELECT *, client_id[*] FROM type::table($table_name) ORDER BY id DESC
);
}
	PERMISSIONS FULL
;
DEFINE FUNCTION OVERWRITE fn::last_n_desc_client($table_name: string, $n: int) {

	fn::check_n($n);

	RETURN (SELECT *, client_id[*] FROM type::table($table_name) ORDER BY id DESC
 LIMIT $n);

}
	PERMISSIONS FULL
;
DEFINE FUNCTION OVERWRITE fn::all_before_key_desc_client($table_name: string, $last_key: string) {
	RETURN (SELECT *, client_id[*] FROM type::thing($table_name, ..=$last_key) ORDER BY id DESC
);
}
	PERMISSIONS FULL
;
DEFINE FUNCTION OVERWRITE fn::last_n_before_key_desc_client($table_name: string, $last_key: string, $n: int) {

	fn::check_n($n);

	RETURN (SELECT *, client_id[*] FROM type::thing($table_name, ..=$last_key) ORDER BY id DESC
 LIMIT $n);

}
	PERMISSIONS FULL
;
